//! GitHub Pages update orchestration, bounded download verification and installation.
//! This module is shared by desktop and the no_std Switch build. Native code
//! supplies only individual HTTP, filesystem, thread and mutex operations.
mod download;
mod files;

use alloc::{format, string::String, sync::Arc, vec};
use core::sync::atomic::{AtomicBool, Ordering};
use serde::{Deserialize, Serialize};

use crate::{
  app::host::{UpdatePhase, UpdateStatus},
  build_identity::BuildIdentity,
  update::{self, UpdateChannel},
};

use super::{
  http::{Http, HttpError, Request, UrlParts},
  runtime::{JoinHandle, Mutex, Runtime},
  storage::Fs,
};

pub use files::{install_update, install_update_with_rename};

const MAX_UPDATE_SIZE: u64 = update::MAX_UPDATE_BYTES as u64;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PendingUpdate {
  pub target_path: String,
  pub staged_path: String,
  pub backup_path: String,
  pub sha256: String,
  pub size: u64,
}

#[derive(Clone, Deserialize)]
struct PublishedUpdate {
  label: String,
  download_url: String,
  size: u64,
  sha256: String,
  notes: String,
}

#[derive(Default)]
struct State {
  status: UpdateStatus,
  pending: Option<PendingUpdate>,
  build: Option<PublishedUpdate>,
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
    state.status.can_download = false;
    state.pending = None;
    self.shared.working.store(true, Ordering::Release);
    Ok(())
  }

  pub fn check(&mut self, repository: &str, channel: UpdateChannel) -> Result<(), String> {
    self.prepare(UpdatePhase::Checking, "Checking the selected GitHub Pages update channel…")?;
    {
      let mut state = self.shared.state.lock();
      state.pending = None;
      state.build = None;
      state.status.version.clear();
      state.status.notes.clear();
      state.status.total = 0;
    }
    let shared = self.shared.clone();
    let repository = String::from(repository);
    match self.runtime.spawn(move || shared.inspect(&repository, channel)) {
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
      if !matches!(
        state.status.phase,
        UpdatePhase::Available | UpdatePhase::Cancelled | UpdatePhase::Error
      ) || state.build.is_none()
      {
        state.status.phase = UpdatePhase::Error;
        state.status.message = "Check for an available update before downloading.".into();
        return Ok(());
      }
      state.build.clone().unwrap()
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
    state.status.can_download = state.build.is_some()
      && matches!(phase, UpdatePhase::Available | UpdatePhase::Cancelled | UpdatePhase::Error);
    self.working.store(false, Ordering::Release);
  }

  fn check_cancelled(&self) -> Result<(), WorkError> {
    if self.cancelled.load(Ordering::Acquire) {
      Err(WorkError::Cancelled)
    } else {
      Ok(())
    }
  }

  fn inspect(&self, repository: &str, channel: UpdateChannel) {
    match self.inspect_build(repository, channel) {
      Ok(Some(build)) => {
        {
          let mut state = self.state.lock();
          state.status.version = build.label.clone();
          state.status.notes = build.notes.clone();
          state.status.total = build.size;
          state.build = Some(build);
        }
        self.finish(UpdatePhase::Available, "An update is available to download and verify.");
      }
      Ok(None) => self.finish(UpdatePhase::Current, "No newer build is available in this channel."),
      Err(WorkError::Cancelled) => self.finish(UpdatePhase::Cancelled, "Update check cancelled."),
      Err(WorkError::Failure(error) | WorkError::Invalid(error)) => {
        self.finish(UpdatePhase::Error, &error)
      }
    }
  }

  fn inspect_build(
    &self,
    repository: &str,
    channel: UpdateChannel,
  ) -> Result<Option<PublishedUpdate>, WorkError> {
    let base = update::pages_base(repository)
      .ok_or_else(|| failure("Set the update repository to owner/repository before checking."))?;
    let mut manifest = alloc::vec::Vec::new();
    self.fetch(
      &format!("{base}/updates/{}", channel.feed_name()),
      MAX_UPDATE_SIZE,
      &mut |bytes| {
        manifest.extend_from_slice(bytes);
        Ok(())
      },
    )?;
    let manifest = core::str::from_utf8(&manifest)
      .map_err(|_| failure("The update source returned invalid version metadata."))?;
    let mut result = update::inspect_channel(
      manifest,
      &self.current_version,
      repository,
      channel,
      BuildIdentity::embedded(),
    );
    if result["ok"].as_bool() != Some(true) {
      return Err(failure(
        result["error"].as_str().unwrap_or("The build does not contain a verified Wodex NRO."),
      ));
    }
    if result["available"].as_bool() != Some(true) {
      self.check_cancelled()?;
      return Ok(None);
    }
    let build: PublishedUpdate = serde_json::from_value(result["build"].take())
      .map_err(|_| failure("Cannot inspect the update build."))?;
    self.check_cancelled()?;
    Ok(Some(build))
  }

  fn transfer(&self, selected: &PublishedUpdate) {
    let result = self.download_build(selected);
    match result {
      Ok(pending) => {
        self.state.lock().pending = Some(pending);
        self.finish(
          UpdatePhase::Ready,
          "Choose Install and exit to apply the verified update, then launch Wodex again.",
        );
      }
      Err(error) => {
        let invalid = matches!(&error, WorkError::Invalid(_));
        let (phase, mut message) = match error {
          WorkError::Cancelled => {
            (UpdatePhase::Cancelled, String::from("Update download cancelled."))
          }
          WorkError::Failure(error) | WorkError::Invalid(error) => (UpdatePhase::Error, error),
        };
        let mut retained = 0;
        if let Ok(candidate) = files::candidate(&self.target_path, &selected.sha256, selected.size)
        {
          if let Ok(Some(info)) = self.fs.stat(&candidate.staged_path, false) {
            if info.is_regular() {
              if invalid || info.size == 0 {
                if self.fs.remove(&candidate.staged_path, true).is_err() {
                  message.push_str(" Cannot remove the incomplete update file.");
                }
              } else if info.size <= selected.size {
                retained = info.size;
              }
            }
          }
        }
        self.state.lock().status.downloaded = retained;
        if retained != 0 {
          message.push_str(" Partial download kept; choose Resume download to retry.");
        }
        self.finish(phase, &message);
      }
    }
  }

  fn request(&self, canonical: &str, status: u16) -> Result<Request, WorkError> {
    self.check_cancelled()?;
    let parsed = pages_url(&self.http, canonical)?;
    let actual = if self.test_api_base.is_empty() {
      String::from(canonical)
    } else {
      format!("{}/{}{}", self.test_api_base, parsed.host, parsed.path)
    };
    Ok(Request {
      url: actual,
      headers: vec!["Accept: application/json, application/octet-stream".into()],
      body: vec![],
      ca_bundle: self.ca_bundle.clone(),
      user_agent: "Wodex-Updater/1".into(),
      protocols: if self.test_api_base.is_empty() { "https" } else { "http" }.into(),
      no_proxy: !self.test_api_base.is_empty(),
      body_status: Some(status),
      abort_rejected_body: true,
      ..Request::default()
    })
  }

  fn fetch(
    &self,
    canonical: &str,
    limit: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
  ) -> Result<(), WorkError> {
    // Both URLs are constructed or checked against an exact project Pages path.
    // Reject redirects entirely: Pages serves these files directly, and a
    // redirect must not be able to transfer trust to another host or project.
    let request = self.request(canonical, 200)?;
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
      return Err(failure(
        "The update server redirected the request. Use the project's direct GitHub Pages URL.",
      ));
    }
    if status != 200 && status != 0 {
      return Err(failure(&match status {
        404 => String::from("No version feed was found. Check the project's GitHub Pages setup."),
        403 | 429 => String::from("The update server refused the request. Try again later."),
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
      return Err(failure("The update server did not return the requested file."));
    }
    Ok(())
  }
}

enum WorkError {
  Cancelled,
  Failure(String),
  Invalid(String),
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

fn pages_url(http: &Http, text: &str) -> Result<UrlParts, String> {
  let parsed = parse_url(http, text)?;
  let owner = parsed.host.strip_suffix(".github.io").unwrap_or_default();
  if parsed.scheme != "https"
    || !parsed.port.is_empty()
    || parsed.has_query
    || !update::valid_repository(&format!("{owner}/project"))
  {
    return Err("Updates may only use direct HTTPS GitHub Pages project URLs.".into());
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
