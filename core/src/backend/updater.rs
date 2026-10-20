//! GitHub update orchestration, bounded download verification and installation.
//! This module is shared by desktop and the no_std Switch build. Native code
//! supplies only individual HTTP, filesystem, thread and mutex operations.
mod files;

use alloc::{format, string::String, sync::Arc, vec};
use core::sync::atomic::{AtomicBool, Ordering};
use serde::{Deserialize, Serialize};

use crate::{
  app::host::{UpdatePhase, UpdateStatus},
  update::{self, WodexSha256},
};

use super::{
  http::{Http, HttpError, Request, UrlParts},
  runtime::{JoinHandle, Mutex, Runtime},
  storage::Fs,
};

pub use files::{install_update, install_update_with_rename};

const MAX_RELEASE_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PendingUpdate {
  pub target_path: String,
  pub staged_path: String,
  pub backup_path: String,
  pub sha256: String,
  pub size: u64,
}

#[derive(Clone, Deserialize)]
struct Release {
  version: String,
  download_url: String,
  size: u64,
  sha256: String,
  notes: String,
}

#[derive(Default)]
struct State {
  status: UpdateStatus,
  pending: Option<PendingUpdate>,
  release: Option<Release>,
}

struct Worker {
  http: Http,
  fs: Fs,
  current_version: String,
  target_path: String,
  test_api_base: String,
  ca_bundle: String,
  cancelled: AtomicBool,
  working: AtomicBool,
  state: Mutex<State>,
}

pub struct Updater {
  runtime: Runtime,
  worker: Option<JoinHandle>,
  shared: Arc<Worker>,
}

impl Updater {
  pub fn new(
    runtime: Runtime,
    http: Http,
    fs: Fs,
    current_version: String,
    target_path: String,
    test_api_base: String,
    ca_bundle: String,
  ) -> Result<Self, String> {
    let test_api_base = validate_test_base(&http, &test_api_base)?;
    Ok(Self {
      runtime,
      worker: None,
      shared: Arc::new(Worker {
        http,
        fs,
        current_version,
        target_path,
        test_api_base,
        ca_bundle,
        cancelled: AtomicBool::new(false),
        working: AtomicBool::new(false),
        state: runtime.mutex(State::default())?,
      }),
    })
  }

  fn prepare(&mut self, phase: UpdatePhase, message: &str) -> Result<(), String> {
    if self.active() {
      return Err("An update operation is already running.".into());
    }
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
    self.shared.cancelled.store(false, Ordering::Release);
    let mut state = self.shared.state.lock();
    state.status.phase = phase;
    state.status.message = message.into();
    state.status.downloaded = 0;
    self.shared.working.store(true, Ordering::Release);
    Ok(())
  }

  pub fn check(&mut self, repository: &str) -> Result<(), String> {
    self.prepare(UpdatePhase::Checking, "Checking the configured GitHub repository…")?;
    {
      let mut state = self.shared.state.lock();
      if let Some(pending) = state.pending.take() {
        let _ = self.shared.fs.remove(&pending.staged_path, true);
      }
      state.release = None;
      state.status.version.clear();
      state.status.notes.clear();
      state.status.total = 0;
    }
    let shared = self.shared.clone();
    let repository = String::from(repository);
    match self.runtime.spawn(move || shared.inspect(&repository)) {
      Ok(worker) => self.worker = Some(worker),
      Err(_) => self.shared.finish(UpdatePhase::Error, "Cannot start the update worker."),
    }
    Ok(())
  }

  pub fn download(&mut self) -> Result<(), String> {
    let selected = {
      let mut state = self.shared.state.lock();
      if self.active() {
        return Err("An update operation is already running.".into());
      }
      if state.status.phase != UpdatePhase::Available || state.release.is_none() {
        state.status.phase = UpdatePhase::Error;
        state.status.message = "Check for an available update before downloading.".into();
        return Ok(());
      }
      state.release.clone().unwrap()
    };
    self.prepare(UpdatePhase::Downloading, "Downloading and verifying the update…")?;
    let shared = self.shared.clone();
    match self.runtime.spawn(move || shared.transfer(&selected)) {
      Ok(worker) => self.worker = Some(worker),
      Err(_) => self.shared.finish(UpdatePhase::Error, "Cannot start the update worker."),
    }
    Ok(())
  }

  pub fn cancel(&self) {
    self.shared.cancelled.store(true, Ordering::Release);
  }

  pub fn active(&self) -> bool {
    self.shared.working.load(Ordering::Acquire)
  }

  pub fn status(&self) -> UpdateStatus {
    self.shared.state.lock().status.clone()
  }

  pub fn pending_install(&self) -> Option<PendingUpdate> {
    self.shared.state.lock().pending.clone()
  }
}

impl Drop for Updater {
  fn drop(&mut self) {
    self.cancel();
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
    // A ready file is an intentional install candidate. Dropping the updater
    // after native resource teardown must leave it available to the installer.
  }
}

impl Worker {
  fn finish(&self, phase: UpdatePhase, message: &str) {
    let mut state = self.state.lock();
    state.status.phase = phase;
    state.status.message = message.into();
    self.working.store(false, Ordering::Release);
  }

  fn check_cancelled(&self) -> Result<(), WorkError> {
    if self.cancelled.load(Ordering::Acquire) {
      Err(WorkError::Cancelled)
    } else {
      Ok(())
    }
  }

  fn inspect(&self, repository: &str) {
    match self.inspect_release(repository) {
      Ok(Some(release)) => {
        {
          let mut state = self.state.lock();
          state.status.version = release.version.clone();
          state.status.notes = release.notes.clone();
          state.status.total = release.size;
          state.release = Some(release);
        }
        self.finish(UpdatePhase::Available, "A newer verified release is available.");
      }
      Ok(None) => self.finish(UpdatePhase::Current, "You are running the latest release."),
      Err(WorkError::Cancelled) => self.finish(UpdatePhase::Cancelled, "Update check cancelled."),
      Err(WorkError::Failure(error)) => self.finish(UpdatePhase::Error, &error),
    }
  }

  fn inspect_release(&self, repository: &str) -> Result<Option<Release>, WorkError> {
    if !update::valid_repository(repository) {
      return Err(failure("Set the update repository to owner/repository before checking."));
    }
    let mut manifest = alloc::vec::Vec::new();
    self.fetch(
      &format!("https://api.github.com/repos/{repository}/releases/latest"),
      MAX_RELEASE_SIZE,
      &mut |bytes| {
        manifest.extend_from_slice(bytes);
        Ok(())
      },
    )?;
    let manifest = core::str::from_utf8(&manifest)
      .map_err(|_| failure("The update source returned invalid release metadata."))?;
    let mut result = update::inspect(manifest, &self.current_version);
    if result["ok"].as_bool() != Some(true) {
      return Err(failure(
        result["error"].as_str().unwrap_or("The release does not contain a verified Wodex NRO."),
      ));
    }
    if result["available"].as_bool() != Some(true) {
      self.check_cancelled()?;
      return Ok(None);
    }
    let release: Release = serde_json::from_value(result["release"].take())
      .map_err(|_| failure("Cannot inspect the update release."))?;
    let asset_url = github_url(&self.http, &release.download_url)?;
    let prefix = format!("/{}/releases/download/", repository.to_ascii_lowercase());
    if !asset_url.path.to_ascii_lowercase().starts_with(&prefix) {
      return Err(failure(
        "The release asset belongs to a different repository. Check the update source.",
      ));
    }
    self.check_cancelled()?;
    Ok(Some(release))
  }

  fn transfer(&self, selected: &Release) {
    let mut created = None;
    let result = self.download_release(selected, &mut created);
    match result {
      Ok(pending) => {
        self.state.lock().pending = Some(pending);
        self.finish(
          UpdatePhase::Ready,
          "Choose Install and exit to apply the verified update, then launch Wodex again.",
        );
      }
      Err(error) => {
        if let Some(staged) = created {
          let _ = self.fs.remove(&staged, true);
        }
        match error {
          WorkError::Cancelled => self.finish(UpdatePhase::Cancelled, "Update download cancelled."),
          WorkError::Failure(error) => self.finish(UpdatePhase::Error, &error),
        }
      }
    }
  }

  fn download_release(
    &self,
    selected: &Release,
    created: &mut Option<String>,
  ) -> Result<PendingUpdate, WorkError> {
    let candidate = files::candidate(&self.target_path, &selected.sha256, selected.size)?;
    if !self.fs.stat(&self.target_path, false)?.is_some_and(|info| info.is_regular()) {
      return Err(failure("The running NRO file is missing or is not a regular file."));
    }
    if let Some(stale) = self.fs.stat(&candidate.staged_path, false)? {
      if !stale.is_regular() || self.fs.remove(&candidate.staged_path, false).is_err() {
        return Err(failure("Cannot remove an incomplete update file."));
      }
    }
    let mut file = self
      .fs
      .create_new(&candidate.staged_path)
      .map_err(|_| failure("Cannot create the update file beside the running NRO."))?;
    *created = Some(candidate.staged_path.clone());
    let mut hash = WodexSha256::new();
    let mut received = 0u64;
    self.fetch(&selected.download_url, candidate.size, &mut |bytes| {
      file
        .write_all(bytes)
        .map_err(|_| String::from("Cannot write the update file. Check free space."))?;
      if !hash.update(bytes) {
        return Err("Cannot verify the update checksum.".into());
      }
      received += bytes.len() as u64;
      self.state.lock().status.downloaded = received;
      Ok(())
    })?;
    if received != candidate.size {
      return Err(failure("The downloaded update size does not match the published release."));
    }
    if hash.finish() != candidate.sha256 {
      return Err(failure("The update checksum does not match the published release."));
    }
    self.check_cancelled()?;
    file.sync().map_err(|_| failure("Cannot flush the update file to storage."))?;
    file.close().map_err(|_| failure("Cannot close the update file."))?;
    files::verify_stage(&self.fs, &candidate)?;
    self.fs.sync_parent(&candidate.staged_path)?;
    self.check_cancelled()?;
    Ok(candidate)
  }

  fn fetch(
    &self,
    canonical: &str,
    limit: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
  ) -> Result<(), WorkError> {
    let mut canonical = String::from(canonical);
    for redirects in 0..=5 {
      self.check_cancelled()?;
      let parsed = github_url(&self.http, &canonical)?;
      let actual = if self.test_api_base.is_empty() {
        canonical.clone()
      } else {
        let mut mapped = format!("{}/{}{}", self.test_api_base, parsed.host, parsed.path);
        if !parsed.query.is_empty() {
          mapped.push('?');
          mapped.push_str(&parsed.query);
        }
        mapped
      };
      let request = Request {
        url: actual,
        headers: vec!["Accept: application/vnd.github+json, application/octet-stream".into()],
        body: vec![],
        ca_bundle: self.ca_bundle.clone(),
        user_agent: "Wodex-Updater/1".into(),
        protocols: if self.test_api_base.is_empty() { "https" } else { "http" }.into(),
        no_proxy: !self.test_api_base.is_empty(),
        body_status: Some(200),
        abort_rejected_body: true,
        ..Request::default()
      };
      let mut received = 0u64;
      let response = self.http.perform(&request, &self.cancelled, &mut |bytes| {
        if bytes.len() as u64 > limit - received {
          return Err("The update response exceeds its expected size.".into());
        }
        sink(bytes)?;
        received += bytes.len() as u64;
        Ok(())
      })?;
      let status = response.http_status;
      if matches!(status, 301 | 302 | 303 | 307 | 308) {
        if redirects == 5 || response.location.is_empty() {
          return Err(failure("The update server returned too many or invalid redirects."));
        }
        validate_url_text(&response.location)?;
        let next = self
          .http
          .resolve_url(&canonical, &response.location)
          .map_err(|_| failure("The update server returned an invalid redirect."))?;
        github_url(&self.http, &next.url)?;
        canonical = next.url;
        continue;
      }
      if status != 200 && status != 0 {
        return Err(failure(&match status {
          404 => {
            String::from("No published release was found. Check the configured public repository.")
          }
          403 | 429 => {
            String::from("GitHub's public update rate limit was reached. Try again later.")
          }
          _ => format!("The update server returned HTTP {status}. Try again later."),
        }));
      }
      if response.curl_code != 0 {
        return Err(failure(if response.curl_code == 60 {
          "The update server's TLS certificate could not be verified."
        } else {
          "The update download failed. Check your network and storage, then try again."
        }));
      }
      if status != 200 {
        return Err(failure("The update server did not return a release."));
      }
      return Ok(());
    }
    Err(failure("The update server returned too many or invalid redirects."))
  }
}

enum WorkError {
  Cancelled,
  Failure(String),
}

impl From<String> for WorkError {
  fn from(error: String) -> Self {
    Self::Failure(error)
  }
}

impl From<HttpError> for WorkError {
  fn from(error: HttpError) -> Self {
    match error {
      HttpError::Cancelled => Self::Cancelled,
      HttpError::Failure(error) => Self::Failure(error),
    }
  }
}

fn failure(message: &str) -> WorkError {
  WorkError::Failure(message.into())
}

fn validate_url_text(text: &str) -> Result<(), String> {
  if text.is_empty()
    || text.len() > 8192
    || text.bytes().any(|byte| byte <= 0x20 || byte == 0x7f || byte == b'\\')
  {
    return Err("The update server supplied an invalid URL.".into());
  }
  Ok(())
}

fn parse_url(http: &Http, text: &str) -> Result<UrlParts, String> {
  validate_url_text(text)?;
  let parsed =
    http.parse_url(text).map_err(|_| String::from("The update server supplied an invalid URL."))?;
  if parsed.has_user || parsed.has_password || parsed.has_fragment {
    return Err("The update URL contains unsupported credentials or fragments.".into());
  }
  Ok(parsed)
}

fn github_url(http: &Http, text: &str) -> Result<UrlParts, String> {
  let parsed = parse_url(http, text)?;
  if parsed.scheme != "https"
    || (!parsed.port.is_empty() && parsed.port != "443")
    || ![
      "api.github.com",
      "github.com",
      "release-assets.githubusercontent.com",
      "objects.githubusercontent.com",
      "github-releases.githubusercontent.com",
    ]
    .contains(&parsed.host.as_str())
  {
    return Err("Updates may only use verified HTTPS GitHub release servers.".into());
  }
  Ok(parsed)
}

fn validate_test_base(http: &Http, text: &str) -> Result<String, String> {
  if text.is_empty() {
    return Ok(String::new());
  }
  let parsed = parse_url(http, text)?;
  if parsed.scheme != "http"
    || !["127.0.0.1", "localhost", "[::1]"].contains(&parsed.host.as_str())
    || !matches!(parsed.path.as_str(), "" | "/")
    || parsed.has_query
  {
    return Err("The update test server must be an explicit HTTP loopback origin.".into());
  }
  Ok(text.strip_suffix('/').unwrap_or(text).into())
}
