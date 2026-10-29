//! One raw partial-file slot, independent of application saves and sidecars.
//! A partial prefix is untrusted until the whole published digest matches.
use super::{failure, files, PendingUpdate, PublishedUpdate, WorkError, Worker};
use crate::{
  backend::{http::ByteRange, storage::File},
  update::WodexSha256,
};
use alloc::{format, string::String};

enum TransferResult {
  Complete,
  Restart,
}

impl Worker {
  pub(super) fn download_build(
    &self,
    selected: &PublishedUpdate,
  ) -> Result<PendingUpdate, WorkError> {
    let candidate = files::candidate(&self.target_path, &selected.sha256, selected.size)?;
    if !self.fs.stat(&self.target_path, false)?.is_some_and(|info| info.is_regular()) {
      return Err(failure("The running NRO file is missing or is not a regular file."));
    }
    let mut restarted = false;
    loop {
      self.check_cancelled()?;
      let mut file = self.open_partial(&candidate)?;
      let mut hash = WodexSha256::new();
      let offset = self.hash_partial(&mut file, &mut hash, candidate.size)?;
      let mut received = offset;
      if offset < candidate.size {
        self.state.lock().status.message = if offset == 0 {
          "Downloading and verifying the update…"
        } else {
          "Resuming and verifying the update…"
        }
        .into();
        let mut oversized = false;
        let result = self.transfer_part(selected, offset, &mut |bytes| {
          if bytes.len() as u64 > candidate.size - received {
            oversized = true;
            return Err("The update response exceeds its expected size.".into());
          }
          file
            .write_all(bytes)
            .map_err(|_| String::from("Cannot write the update file. Check free space."))?;
          if !hash.update(bytes) {
            return Err("Cannot verify the update checksum.".into());
          }
          received += bytes.len() as u64;
          self.state.lock().status.downloaded = received;
          Ok(())
        });
        if oversized {
          return Err(WorkError::Invalid("The update response exceeds its expected size.".into()));
        }
        match result {
          Ok(TransferResult::Restart) => {
            file.close()?;
            if restarted {
              return Err(failure("The update server could not resume the download."));
            }
            self.reset_partial(&candidate)?;
            restarted = true;
            continue;
          }
          Err(error) => {
            // Flush on the worker, retaining useful bytes across app restarts.
            // A failed flush never makes this partial eligible for installation.
            let _ = file.sync();
            let _ = file.close();
            return Err(error);
          }
          Ok(TransferResult::Complete) => {}
        }
      }
      self.check_cancelled().inspect_err(|_| {
        let _ = file.sync();
      })?;
      if received != candidate.size {
        return Err(WorkError::Invalid(
          "The downloaded update size does not match the published build.".into(),
        ));
      }
      if hash.finish() != candidate.sha256 {
        file.close()?;
        // A single fixed filename needs no metadata sidecar. A stale or
        // damaged prefix can cost one attempt, but cannot pass final integrity.
        if offset != 0 && !restarted {
          self.reset_partial(&candidate)?;
          restarted = true;
          continue;
        }
        return Err(WorkError::Invalid(
          "The update checksum does not match the published build.".into(),
        ));
      }
      if file.size()? != candidate.size {
        return Err(WorkError::Invalid("The update file changed during verification.".into()));
      }
      self.check_cancelled().inspect_err(|_| {
        let _ = file.sync();
      })?;
      file.sync().map_err(|_| failure("Cannot flush the update file to storage."))?;
      file.close().map_err(|_| failure("Cannot close the update file."))?;
      // Reopen the persisted bytes: newly created files are write-only, and
      // neither a stream hash nor a valid header proves the SD write was sound.
      files::verify_stage_checked(&self.fs, &candidate, || self.check_cancelled())?;
      self.fs.sync_parent(&candidate.staged_path)?;
      self.check_cancelled()?;
      return Ok(candidate);
    }
  }

  fn open_partial(&self, pending: &PendingUpdate) -> Result<File, WorkError> {
    match self.fs.stat(&pending.staged_path, false)? {
      Some(info) if !info.is_regular() => {
        Err(failure("The update temporary path is not a regular file. Nothing was changed."))
      }
      Some(info) if info.size > pending.size => {
        self.reset_partial(pending)?;
        self.fs.create_new(&pending.staged_path).map_err(WorkError::from)
      }
      Some(_) => self.fs.open_read_write(&pending.staged_path).map_err(WorkError::from),
      None => self
        .fs
        .create_new(&pending.staged_path)
        .map_err(|_| failure("Cannot create the update file beside the running NRO.")),
    }
  }

  fn reset_partial(&self, pending: &PendingUpdate) -> Result<(), WorkError> {
    self.check_cancelled()?;
    if let Some(info) = self.fs.stat(&pending.staged_path, false)? {
      if !info.is_regular() {
        return Err(failure(
          "The update temporary path is not a regular file. Nothing was changed.",
        ));
      }
      self.fs.remove(&pending.staged_path, false)?;
    }
    self.state.lock().status.downloaded = 0;
    Ok(())
  }

  fn hash_partial(
    &self,
    file: &mut File,
    hash: &mut WodexSha256,
    limit: u64,
  ) -> Result<u64, WorkError> {
    let size = file.size()?;
    if size > limit {
      return Err(failure("The partial download changed size. Try again."));
    }
    if size != 0 {
      self.state.lock().status.message = "Checking the saved partial download…".into();
    }
    let mut buffer = [0u8; 16 * 1024];
    let mut received = 0;
    while received < size {
      self.check_cancelled()?;
      let length = (size - received).min(buffer.len() as u64) as usize;
      let count = file.read(&mut buffer[..length])?;
      if count == 0 || !hash.update(&buffer[..count]) {
        return Err(failure("Cannot verify the saved partial download."));
      }
      received += count as u64;
      self.state.lock().status.downloaded = received;
    }
    self.check_cancelled()?;
    if file.size()? != size {
      return Err(failure("The partial download changed during verification. Try again."));
    }
    file.seek(size)?;
    Ok(size)
  }

  fn transfer_part(
    &self,
    selected: &PublishedUpdate,
    offset: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
  ) -> Result<TransferResult, WorkError> {
    let expected_status = if offset == 0 { 200 } else { 206 };
    let mut request = self.request(&selected.download_url, expected_status)?;
    request.headers.push("Accept-Encoding: identity".into());
    if offset != 0 {
      request.headers.push(format!("Range: bytes={offset}-"));
      request.expected_range = Some(ByteRange { start: offset, total: selected.size });
    }
    let response = self.http.perform(&request, &self.cancelled, sink)?;
    if offset != 0 && matches!(response.http_status, 200 | 416) {
      return Ok(TransferResult::Restart);
    }
    if matches!(response.http_status, 301 | 302 | 303 | 307 | 308) {
      return Err(failure(
        "The update server redirected the request. Use the project's direct GitHub Pages URL.",
      ));
    }
    if response.http_status != expected_status {
      return Err(failure("The update server did not return the requested file. Try again later."));
    }
    if response.curl_code != 0 {
      return Err(failure(if response.curl_code == 60 {
        "The update server's TLS certificate could not be verified."
      } else {
        "The update download was interrupted. Check your network and storage, then retry."
      }));
    }
    Ok(TransferResult::Complete)
  }
}
