//! Platform services behind a borrowed C callback table. Application and input
//! decisions remain in Rust; callbacks only translate native events and perform I/O.
use alloc::{string::String, vec::Vec};
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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeTransportEvent {
  pub kind: u32,
  pub data: Bytes,
  pub http_status: u16,
  pub error: Bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeUpdateStatus {
  pub phase: u32,
  pub message: Bytes,
  pub version: Bytes,
  pub notes: Bytes,
  pub downloaded: u64,
  pub total: u64,
}

/// All callbacks are nonthrowing. Returned slices remain valid until the next
/// callback on this context. The application must not use a table concurrently.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
  pub context: *mut c_void,
  pub now_ms: unsafe extern "C" fn(*mut c_void) -> u64,
  pub running: unsafe extern "C" fn(*mut c_void) -> bool,
  pub is_switch: unsafe extern "C" fn(*mut c_void) -> bool,
  pub load: unsafe extern "C" fn(*mut c_void, *mut Bytes) -> bool,
  pub save: unsafe extern "C" fn(*mut c_void, Bytes) -> bool,
  pub preserve: unsafe extern "C" fn(*mut c_void, Bytes) -> bool,
  pub environment_key: unsafe extern "C" fn(*mut c_void) -> Bytes,
  pub start: unsafe extern "C" fn(*mut c_void, Bytes, Bytes, Bytes) -> bool,
  pub cancel: unsafe extern "C" fn(*mut c_void),
  pub network_active: unsafe extern "C" fn(*mut c_void) -> bool,
  pub poll_transport: unsafe extern "C" fn(*mut c_void) -> NativeTransportEvent,
  pub update_status: unsafe extern "C" fn(*mut c_void) -> NativeUpdateStatus,
  pub update_supported: unsafe extern "C" fn(*mut c_void) -> bool,
  pub update_active: unsafe extern "C" fn(*mut c_void) -> bool,
  pub update_check: unsafe extern "C" fn(*mut c_void, Bytes) -> bool,
  pub update_download: unsafe extern "C" fn(*mut c_void) -> bool,
  pub update_cancel: unsafe extern "C" fn(*mut c_void),
  pub prepare_install: unsafe extern "C" fn(*mut c_void, *mut bool) -> bool,
  pub begin_frame: unsafe extern "C" fn(*mut c_void),
  pub poll_input: unsafe extern "C" fn(*mut c_void) -> NativeInput,
  pub text_input: unsafe extern "C" fn(*mut c_void, bool),
  pub clipboard: unsafe extern "C" fn(*mut c_void) -> Bytes,
  pub edit_system: unsafe extern "C" fn(*mut c_void, Bytes, Bytes, bool, *mut Bytes) -> bool,
  pub last_error: unsafe extern "C" fn(*mut c_void) -> Bytes,
}

pub struct NativeHost {
  api: HostApi,
}

impl NativeHost {
  /// # Safety
  /// Every callback and its context must remain valid until this host is dropped.
  /// Callbacks must implement the borrowing and nonthrowing contract of HostApi.
  pub unsafe fn new(api: &HostApi) -> Self {
    Self { api: *api }
  }

  fn checked(&self, success: bool) -> Result<(), String> {
    if success {
      Ok(())
    } else {
      Err(unsafe { (self.api.last_error)(self.api.context).string() })
    }
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
    unsafe { (self.api.is_switch)(self.api.context) }
  }
  fn load(&mut self) -> Result<Vec<u8>, String> {
    let mut output = Bytes::empty();
    self.checked(unsafe { (self.api.load)(self.api.context, &mut output) })?;
    Ok(unsafe { output.bytes() }.to_vec())
  }
  fn save(&mut self, content: &str) -> Result<(), String> {
    self.checked(unsafe { (self.api.save)(self.api.context, Bytes::from_str(content)) })
  }
  fn preserve(&mut self, content: &[u8]) -> Result<(), String> {
    self.checked(unsafe {
      (self.api.preserve)(self.api.context, Bytes { data: content.as_ptr(), len: content.len() })
    })
  }
  fn environment_key(&self) -> String {
    unsafe { (self.api.environment_key)(self.api.context).string() }
  }
  fn start(&mut self, url: &str, key: &str, body: &str) -> Result<(), String> {
    self.checked(unsafe {
      (self.api.start)(
        self.api.context,
        Bytes::from_str(url),
        Bytes::from_str(key),
        Bytes::from_str(body),
      )
    })
  }
  fn cancel(&mut self) {
    unsafe { (self.api.cancel)(self.api.context) }
  }
  fn network_active(&self) -> bool {
    unsafe { (self.api.network_active)(self.api.context) }
  }
  fn poll_transport(&mut self) -> Option<TransportEvent> {
    let event = unsafe { (self.api.poll_transport)(self.api.context) };
    match event.kind {
      1 => Some(TransportEvent::Data(unsafe { event.data.bytes() }.to_vec())),
      2 => Some(TransportEvent::Finished {
        http_status: event.http_status,
        error: unsafe { event.error.string() },
      }),
      _ => None,
    }
  }
  fn update_status(&self) -> UpdateStatus {
    let status = unsafe { (self.api.update_status)(self.api.context) };
    UpdateStatus {
      phase: match status.phase {
        0 => UpdatePhase::Idle,
        1 => UpdatePhase::Checking,
        2 => UpdatePhase::Current,
        3 => UpdatePhase::Available,
        4 => UpdatePhase::Downloading,
        5 => UpdatePhase::Ready,
        6 => UpdatePhase::Cancelled,
        _ => UpdatePhase::Error,
      },
      message: unsafe { status.message.string() },
      version: unsafe { status.version.string() },
      notes: unsafe { status.notes.string() },
      downloaded: status.downloaded,
      total: status.total,
    }
  }
  fn update_supported(&self) -> bool {
    unsafe { (self.api.update_supported)(self.api.context) }
  }
  fn update_active(&self) -> bool {
    unsafe { (self.api.update_active)(self.api.context) }
  }
  fn update_check(&mut self, repository: &str) -> Result<(), String> {
    self.checked(unsafe { (self.api.update_check)(self.api.context, Bytes::from_str(repository)) })
  }
  fn update_download(&mut self) -> Result<(), String> {
    self.checked(unsafe { (self.api.update_download)(self.api.context) })
  }
  fn update_cancel(&mut self) {
    unsafe { (self.api.update_cancel)(self.api.context) }
  }
  fn prepare_install(&mut self) -> Result<bool, String> {
    let mut ready = false;
    self.checked(unsafe { (self.api.prepare_install)(self.api.context, &mut ready) })?;
    Ok(ready)
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
