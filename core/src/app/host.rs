//! Platform services behind a borrowed C callback table. Application and input
//! decisions remain in Rust; callbacks only translate native events and perform I/O.
use crate::backend::{
  http::Http,
  runtime::Runtime,
  storage::Fs,
  transport::Transport,
  updater::{PendingUpdate, Updater},
  BackendApi,
};
use alloc::{collections::VecDeque, format, string::String, vec::Vec};
use core::{ffi::c_void, slice};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Key {
  Escape = 1,
  Enter,
  Space,
  Left,
  Right,
  Up,
  Down,
  Backspace,
  Delete,
  Home,
  End,
  PageUp,
  PageDown,
  F1,
  Minus,
  X,
  Y,
  A,
  V,
  U,
  LeftBracket,
  RightBracket,
}

impl Key {
  fn from_native(value: u32) -> Option<Self> {
    Some(match value {
      1 => Self::Escape,
      2 => Self::Enter,
      3 => Self::Space,
      4 => Self::Left,
      5 => Self::Right,
      6 => Self::Up,
      7 => Self::Down,
      8 => Self::Backspace,
      9 => Self::Delete,
      10 => Self::Home,
      11 => Self::End,
      12 => Self::PageUp,
      13 => Self::PageDown,
      14 => Self::F1,
      15 => Self::Minus,
      16 => Self::X,
      17 => Self::Y,
      18 => Self::A,
      19 => Self::V,
      20 => Self::U,
      21 => Self::LeftBracket,
      22 => Self::RightBracket,
      _ => return None,
    })
  }
}

pub const SHIFT: u32 = 1;
pub const CTRL: u32 = 2;
pub const GUI: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
  Quit,
  Key { key: Key, modifiers: u32 },
  Text(String),
  Composition(String),
  Pointer { x: i32, y: i32 },
  Scroll(i32),
  Gamepad(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportEvent {
  Data(Vec<u8>),
  Finished { http_status: u16, error: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdatePhase {
  #[default]
  Idle,
  Checking,
  Current,
  Available,
  Downloading,
  Ready,
  Cancelled,
  Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateStatus {
  pub phase: UpdatePhase,
  pub message: String,
  pub version: String,
  pub notes: String,
  pub downloaded: u64,
  pub total: u64,
}

pub trait Host {
  fn now_ms(&self) -> u64;
  fn running(&self) -> bool;
  fn is_switch(&self) -> bool;
  fn load(&mut self) -> Result<Vec<u8>, String>;
  fn save(&mut self, content: &str) -> Result<(), String>;
  fn preserve(&mut self, content: &[u8]) -> Result<(), String>;
  fn environment_key(&self) -> String;
  fn start(&mut self, url: &str, key: &str, body: &str) -> Result<(), String>;
  fn cancel(&mut self);
  fn network_active(&self) -> bool;
  fn poll_transport(&mut self) -> Option<TransportEvent>;
  fn update_status(&self) -> UpdateStatus;
  fn update_supported(&self) -> bool;
  fn update_active(&self) -> bool;
  fn update_check(&mut self, repository: &str) -> Result<(), String>;
  fn update_download(&mut self) -> Result<(), String>;
  fn update_cancel(&mut self);
  fn prepare_install(&mut self) -> Result<bool, String>;
  fn begin_frame(&mut self);
  fn poll_input(&mut self) -> Option<Input>;
  fn text_input(&mut self, enabled: bool);
  fn clipboard(&mut self) -> String;
  fn edit_system(&mut self, title: &str, initial: &str, secret: bool) -> Result<String, String>;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
  pub data: *const u8,
  pub len: usize,
}

impl Bytes {
  fn empty() -> Self {
    Self { data: core::ptr::null(), len: 0 }
  }

  fn from_str(value: &str) -> Self {
    Self { data: value.as_ptr(), len: value.len() }
  }

  unsafe fn bytes(&self) -> &[u8] {
    if self.len == 0 {
      &[]
    } else {
      slice::from_raw_parts(self.data, self.len)
    }
  }

  unsafe fn string(&self) -> String {
    String::from_utf8_lossy(self.bytes()).into_owned()
  }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeInput {
  pub kind: u32,
  pub key: u32,
  pub modifiers: u32,
  pub x: i32,
  pub y: i32,
  pub value: i32,
  pub text: Bytes,
}

/// Native callbacks expose platform and SDL bindings only. Storage, HTTP and
/// update policy are Rust-owned and use separate primitive backend tables.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
  pub context: *mut c_void,
  pub now_ms: unsafe extern "C" fn(*mut c_void) -> u64,
  pub running: unsafe extern "C" fn(*mut c_void) -> bool,
  pub begin_frame: unsafe extern "C" fn(*mut c_void),
  pub poll_input: unsafe extern "C" fn(*mut c_void) -> NativeInput,
  pub text_input: unsafe extern "C" fn(*mut c_void, bool),
  pub clipboard: unsafe extern "C" fn(*mut c_void) -> Bytes,
  pub edit_system: unsafe extern "C" fn(*mut c_void, Bytes, Bytes, bool, *mut Bytes) -> bool,
  pub last_error: unsafe extern "C" fn(*mut c_void) -> Bytes,
}

pub struct NativeHost {
  api: HostApi,
  fs: Fs,
  save_path: String,
  transport: Transport,
  updater: Updater,
  transport_events: VecDeque<TransportEvent>,
  install_supported: bool,
  pending: Option<PendingUpdate>,
}

/// The loader path must designate the current SD-card executable, including
/// when launched from a nondefault directory. Backup NROs stay recovery-only.
fn install_target(path: &str, is_switch: bool) -> String {
  if !is_switch
    || !(11..=768).contains(&path.len())
    || !path.starts_with("sdmc:/")
    || !path.ends_with(".nro")
    || path.ends_with(".previous.nro")
    || path.contains('\\')
    || path.contains("//")
    || path.contains("/../")
    || path.contains("/./")
    || path.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
  {
    String::new()
  } else {
    path.into()
  }
}

impl NativeHost {
  /// # Safety
  /// All tables and native contexts must outlive this host. The backend callbacks
  /// must be nonthrowing and support their documented worker-thread use.
  pub unsafe fn new(
    api: &HostApi,
    backend: &BackendApi,
    version: &str,
    target: &str,
  ) -> Result<Self, String> {
    let fs = Fs::new(&backend.fs);
    let directory = fs.data_dir()?;
    let runtime = Runtime::new(&backend.runtime);
    let http = Http::new(&backend.http)?;
    let target = install_target(target, fs.is_switch());
    let install_supported = !target.is_empty();
    let ca_bundle =
      if fs.is_switch() { "romfs:/cacert.pem".into() } else { fs.environment("WODEX_CA_BUNDLE") };
    let transport = Transport::new(runtime, http, ca_bundle.clone())?;
    let updater =
      Updater::new(runtime, http, fs, version.into(), target, String::new(), ca_bundle)?;
    Ok(Self {
      api: *api,
      fs,
      save_path: format!("{directory}/state.json"),
      transport,
      updater,
      transport_events: VecDeque::new(),
      install_supported,
      pending: None,
    })
  }

  pub fn take_pending(&mut self) -> Option<PendingUpdate> {
    self.pending.take()
  }

  fn checked(&self, success: bool) -> Result<(), String> {
    if success {
      Ok(())
    } else {
      Err(unsafe { (self.api.last_error)(self.api.context).string() })
    }
  }
}

impl Drop for NativeHost {
  fn drop(&mut self) {
    self.transport.cancel();
    self.updater.cancel();
    // Each Rust backend joins its own worker before its shared data is dropped.
  }
}

impl Host for NativeHost {
  fn now_ms(&self) -> u64 {
    unsafe { (self.api.now_ms)(self.api.context) }
  }
  fn running(&self) -> bool {
    unsafe { (self.api.running)(self.api.context) }
  }
  fn is_switch(&self) -> bool {
    self.fs.is_switch()
  }
  fn load(&mut self) -> Result<Vec<u8>, String> {
    self.fs.load_file(&self.save_path)
  }
  fn save(&mut self, content: &str) -> Result<(), String> {
    self.fs.atomic_write(&self.save_path, content.as_bytes())
  }
  fn preserve(&mut self, content: &[u8]) -> Result<(), String> {
    self.fs.preserve_file(&self.save_path, content).map(|_| ())
  }
  fn environment_key(&self) -> String {
    self.fs.environment("DEEPSEEK_API_KEY")
  }
  fn start(&mut self, url: &str, key: &str, body: &str) -> Result<(), String> {
    self.transport.start(url, key, body)
  }
  fn cancel(&mut self) {
    self.transport.cancel();
  }
  fn network_active(&self) -> bool {
    self.transport.active()
  }
  fn poll_transport(&mut self) -> Option<TransportEvent> {
    if self.transport_events.is_empty() {
      self.transport_events.extend(self.transport.poll());
    }
    self.transport_events.pop_front()
  }
  fn update_status(&self) -> UpdateStatus {
    self.updater.status()
  }
  fn update_supported(&self) -> bool {
    self.install_supported
  }
  fn update_active(&self) -> bool {
    self.updater.active()
  }
  fn update_check(&mut self, repository: &str) -> Result<(), String> {
    self.updater.check(repository)
  }
  fn update_download(&mut self) -> Result<(), String> {
    self.updater.download()
  }
  fn update_cancel(&mut self) {
    self.updater.cancel();
  }
  fn prepare_install(&mut self) -> Result<bool, String> {
    if self.transport.active() {
      return Err("Wait for the response to finish before installing.".into());
    }
    self.pending = self.updater.pending_install();
    Ok(self.install_supported && self.pending.is_some())
  }
  fn begin_frame(&mut self) {
    unsafe { (self.api.begin_frame)(self.api.context) }
  }
  fn poll_input(&mut self) -> Option<Input> {
    let event = unsafe { (self.api.poll_input)(self.api.context) };
    match event.kind {
      1 => Some(Input::Quit),
      2 => Some(Input::Key { key: Key::from_native(event.key)?, modifiers: event.modifiers }),
      3 => Some(Input::Text(unsafe { event.text.string() })),
      4 => Some(Input::Composition(unsafe { event.text.string() })),
      5 => Some(Input::Pointer { x: event.x, y: event.y }),
      6 => Some(Input::Scroll(event.value)),
      7 => Some(Input::Gamepad(event.value as u8)),
      _ => None,
    }
  }
  fn text_input(&mut self, enabled: bool) {
    unsafe { (self.api.text_input)(self.api.context, enabled) }
  }
  fn clipboard(&mut self) -> String {
    unsafe { (self.api.clipboard)(self.api.context).string() }
  }
  fn edit_system(&mut self, title: &str, initial: &str, secret: bool) -> Result<String, String> {
    if self.fs.is_switch() && initial.encode_utf16().count() > 249 {
      return Err("The system keyboard cannot reopen a draft longer than 249 characters. Send or clear this draft first.".into());
    }
    let mut output = Bytes::empty();
    self.checked(unsafe {
      (self.api.edit_system)(
        self.api.context,
        Bytes::from_str(title),
        Bytes::from_str(initial),
        secret,
        &mut output,
      )
    })?;
    Ok(unsafe { output.string() })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn installation_target_is_the_loader_sd_executable_and_excludes_recovery_paths() {
    assert_eq!(
      install_target("sdmc:/switch/custom/wodex.nro", true),
      "sdmc:/switch/custom/wodex.nro"
    );
    for invalid in [
      "sdmc:/switch/wodex.previous.nro",
      "sdmc:/switch/../wodex.nro",
      "sdmc://wodex.nro",
      "sdmc:/switch/./wodex.nro",
      "/tmp/wodex.nro",
      "sdmc:/switch/wodex.nro\n",
    ] {
      assert!(install_target(invalid, true).is_empty());
    }
    assert!(install_target("sdmc:/switch/wodex.nro", false).is_empty());
  }
}
