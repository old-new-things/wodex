//! Wodex-only HOME Menu installation. Rust validates the bundled key-free
//! template and owns all content, metadata, rollback and worker decisions.
mod package;
#[cfg(test)]
mod tests;

use super::{
  runtime::{JoinHandle, Mutex, Runtime},
  storage::{File, Fs},
};
use alloc::{format, string::String, sync::Arc, vec};
use core::{
  ffi::c_void,
  sync::atomic::{AtomicBool, Ordering},
};
use package::Package;

pub(crate) const TITLE_ID: u64 = 0x0577_6f64_6578_0000;
pub(crate) const NRO_PATH: &str = "sdmc:/switch/wodex/wodex.nro";
const TEMPLATE_PATH: &str = "romfs:/home-menu/template.bin";
const MAX_TEMPLATE_SIZE: usize = 16 * 1024 * 1024;
const COPY_CHUNK: usize = 64 * 1024;
const FREE_SPACE_MARGIN: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HomeMenuPhase {
  #[default]
  Unsupported,
  Idle,
  Checking,
  Ready,
  Installing,
  Installed,
  Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HomeMenuStatus {
  pub phase: HomeMenuPhase,
  pub message: String,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetaKey {
  id: u64,
  version: u32,
  kind: u8,
  install_type: u8,
  reserved: [u8; 2],
}

const META_KEY: MetaKey =
  MetaKey { id: TITLE_ID, version: 0, kind: 0x80, install_type: 0, reserved: [0; 2] };

/// See wodex_home_menu.h. Every Result is zero on success. Buffers are borrowed
/// only for the duration of a call; callbacks must never unwind or retain them.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HomeMenuApi {
  pub open: unsafe extern "C" fn(*mut *mut c_void) -> u32,
  pub close: unsafe extern "C" fn(*mut c_void),
  pub application_records: unsafe extern "C" fn(*mut c_void, u32, *mut u64, u32, *mut u32) -> u32,
  pub metadata_exists: unsafe extern "C" fn(*mut c_void, u8, u64, *mut bool) -> u32,
  pub content_exists: unsafe extern "C" fn(*mut c_void, *const u8, *mut bool) -> u32,
  pub free_space: unsafe extern "C" fn(*mut c_void, *mut u64) -> u32,
  pub seal_header: unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> u32,
  pub set_installing: unsafe extern "C" fn(*mut c_void, bool) -> u32,
  pub placeholder_id: unsafe extern "C" fn(*mut c_void, *mut u8) -> u32,
  pub placeholder_exists: unsafe extern "C" fn(*mut c_void, *const u8, *mut bool) -> u32,
  pub placeholder_create: unsafe extern "C" fn(*mut c_void, *const u8, *const u8, u64) -> u32,
  pub placeholder_write: unsafe extern "C" fn(*mut c_void, *const u8, u64, *const u8, usize) -> u32,
  pub placeholder_flush: unsafe extern "C" fn(*mut c_void) -> u32,
  pub placeholder_register: unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> u32,
  pub placeholder_delete: unsafe extern "C" fn(*mut c_void, *const u8) -> u32,
  pub content_delete: unsafe extern "C" fn(*mut c_void, *const u8) -> u32,
  pub metadata_set: unsafe extern "C" fn(*mut c_void, *const MetaKey, *const u8, usize) -> u32,
  pub metadata_remove: unsafe extern "C" fn(*mut c_void, *const MetaKey) -> u32,
  pub metadata_commit: unsafe extern "C" fn(*mut c_void) -> u32,
  pub application_register: unsafe extern "C" fn(*mut c_void, *const MetaKey) -> u32,
}

fn checked(result: u32, operation: &str) -> Result<(), String> {
  if result == 0 {
    Ok(())
  } else {
    Err(format!("{operation} (0x{result:08X})."))
  }
}

struct Session {
  api: HomeMenuApi,
  handle: *mut c_void,
}

impl Session {
  fn open(api: HomeMenuApi) -> Result<Self, String> {
    let mut handle = core::ptr::null_mut();
    checked(unsafe { (api.open)(&mut handle) }, "Cannot open HOME Menu installation services")?;
    if handle.is_null() {
      return Err("HOME Menu installation services returned an invalid session.".into());
    }
    Ok(Self { api, handle })
  }

  fn has_application(&self) -> Result<bool, String> {
    let mut ids = [0u64; 32];
    // Bound pagination even if a faulty adapter keeps returning a full page.
    for page in 0..4096 {
      let mut count = 0;
      checked(
        unsafe {
          (self.api.application_records)(self.handle, page * 32, ids.as_mut_ptr(), 32, &mut count)
        },
        "Cannot inspect existing HOME Menu titles",
      )?;
      if count > 32 {
        return Err("The HOME Menu title list is invalid.".into());
      }
      if ids[..count as usize].contains(&TITLE_ID) {
        return Ok(true);
      }
      if count < 32 {
        return Ok(false);
      }
    }
    Err("The HOME Menu title list exceeds the supported size.".into())
  }

  fn has_metadata(&self, storage: u8) -> Result<bool, String> {
    let mut exists = false;
    checked(
      unsafe { (self.api.metadata_exists)(self.handle, storage, TITLE_ID, &mut exists) },
      "Cannot inspect existing title metadata",
    )?;
    Ok(exists)
  }

  fn occupied(&self) -> Result<bool, String> {
    Ok(self.has_application()? || self.has_metadata(5)? || self.has_metadata(4)?)
  }

  fn has_content(&self, id: &[u8; 16]) -> Result<bool, String> {
    let mut exists = false;
    checked(
      unsafe { (self.api.content_exists)(self.handle, id.as_ptr(), &mut exists) },
      "Cannot inspect existing title content",
    )?;
    Ok(exists)
  }

  fn has_placeholder(&self, id: &[u8; 16]) -> Result<bool, String> {
    let mut exists = false;
    checked(
      unsafe { (self.api.placeholder_exists)(self.handle, id.as_ptr(), &mut exists) },
      "Cannot inspect an installation placeholder",
    )?;
    Ok(exists)
  }

  fn free_space(&self, required: u64) -> Result<(), String> {
    let mut available = 0;
    checked(
      unsafe { (self.api.free_space)(self.handle, &mut available) },
      "Cannot check SD card free space",
    )?;
    if available < required.saturating_add(FREE_SPACE_MARGIN) {
      return Err("Not enough free SD card space to install the HOME Menu shortcut.".into());
    }
    Ok(())
  }

  fn seal(&self, data: &mut [u8]) -> Result<(), String> {
    checked(
      unsafe { (self.api.seal_header)(self.handle, data.as_mut_ptr(), data.len()) },
      "Cannot seal the shortcut on this console",
    )
  }
}

impl Drop for Session {
  fn drop(&mut self) {
    unsafe { (self.api.close)(self.handle) };
  }
}

struct Shared {
  fs: Fs,
  api: Option<HomeMenuApi>,
  working: AtomicBool,
  status: Mutex<HomeMenuStatus>,
}

pub struct HomeMenu {
  runtime: Runtime,
  worker: Option<JoinHandle>,
  shared: Arc<Shared>,
}

impl HomeMenu {
  /// # Safety
  /// The optional table must remain valid during this call; callbacks are copied
  /// and must remain executable until this object and its worker are dropped.
  pub unsafe fn new(
    runtime: Runtime,
    fs: Fs,
    api: *const HomeMenuApi,
    target: String,
  ) -> Result<Self, String> {
    let (api, status) = if !fs.is_switch() || api.is_null() {
      (
        None,
        HomeMenuStatus {
          phase: HomeMenuPhase::Unsupported,
          message: "HOME Menu shortcuts are available on Nintendo Switch.".into(),
        },
      )
    } else if target != NRO_PATH {
      (
        None,
        HomeMenuStatus {
          phase: HomeMenuPhase::Unsupported,
          message:
            "Move Wodex to sdmc:/switch/wodex/wodex.nro, launch it again, then install the shortcut."
              .into(),
        },
      )
    } else {
      (
        Some(unsafe { *api }),
        HomeMenuStatus {
          phase: HomeMenuPhase::Idle,
          message: "Check whether this console can install the HOME Menu shortcut.".into(),
        },
      )
    };
    Ok(Self {
      runtime,
      worker: None,
      shared: Arc::new(Shared {
        fs,
        api,
        working: AtomicBool::new(false),
        status: runtime.mutex(status)?,
      }),
    })
  }

  pub fn active(&self) -> bool {
    self.shared.working.load(Ordering::Acquire)
  }

  pub fn status(&self) -> HomeMenuStatus {
    self.shared.status.lock().clone()
  }

  pub fn check(&mut self) -> Result<(), String> {
    self.start(false)
  }

  pub fn install(&mut self) -> Result<(), String> {
    if self.status().phase != HomeMenuPhase::Ready {
      return Err("Check HOME Menu installation support first.".into());
    }
    self.start(true)
  }

  fn start(&mut self, install: bool) -> Result<(), String> {
    if self.active() {
      return Err("A HOME Menu operation is already running.".into());
    }
    if self.shared.api.is_none() {
      return Err(self.status().message);
    }
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
    {
      let mut status = self.shared.status.lock();
      status.phase = if install { HomeMenuPhase::Installing } else { HomeMenuPhase::Checking };
      status.message = if install {
        "Installing the HOME Menu shortcut. Keep Wodex open and leave the SD card inserted.".into()
      } else {
        "Checking HOME Menu installation support…".into()
      };
    }
    self.shared.working.store(true, Ordering::Release);
    let shared = Arc::clone(&self.shared);
    match self.runtime.spawn(move || shared.run(install)) {
      Ok(worker) => {
        self.worker = Some(worker);
        Ok(())
      }
      Err(_) => {
        let message = "Cannot start the HOME Menu worker.";
        self.shared.finish(HomeMenuPhase::Error, message.into());
        Err(message.into())
      }
    }
  }
}

impl Drop for HomeMenu {
  fn drop(&mut self) {
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
  }
}

impl Shared {
  fn finish(&self, phase: HomeMenuPhase, message: String) {
    *self.status.lock() = HomeMenuStatus { phase, message };
    self.working.store(false, Ordering::Release);
  }

  fn run(&self, install: bool) {
    match self.execute(install) {
      Ok(status) => self.finish(status.phase, status.message),
      Err(message) => self.finish(HomeMenuPhase::Error, message),
    }
  }

  fn load_package(&self) -> Result<Package, String> {
    let mut nro = self.fs.open_read(NRO_PATH).map_err(|_| {
      String::from("Cannot read sdmc:/switch/wodex/wodex.nro. Restore Wodex there and retry.")
    })?;
    let mut header = [0u8; 0x80];
    read_exact(&mut nro, &mut header)?;
    if &header[0x10..0x14] != b"NRO0" {
      return Err("The shortcut target is not a valid NRO executable.".into());
    }
    let mut file = self.fs.open_read(TEMPLATE_PATH).map_err(|_| {
      String::from("This Wodex build does not include a HOME Menu shortcut. Install a current official nightly.")
    })?;
    let size = file.info()?.size;
    if !(20..=MAX_TEMPLATE_SIZE as u64).contains(&size) {
      return Err("The bundled HOME Menu shortcut has an invalid size.".into());
    }
    let mut data = vec![0u8; size as usize];
    read_exact(&mut file, &mut data)?;
    verify_bundle(&data)?;
    Package::parse(data)
  }

  fn execute(&self, install: bool) -> Result<HomeMenuStatus, String> {
    let api = self.api.ok_or("HOME Menu installation is unavailable.")?;
    let mut package = self.load_package()?;
    let session = Session::open(api)?;
    if session.occupied()? {
      return Ok(HomeMenuStatus {
        phase: HomeMenuPhase::Installed,
        message: "A HOME Menu title already uses Wodex's identifier. The existing title was left unchanged.".into(),
      });
    }
    session.free_space(package.size() as u64)?;
    package.seal(|header| session.seal(header))?;
    for id in package.ids() {
      if session.has_content(id)? {
        return Err("Shortcut content already exists without a HOME Menu entry. No existing content was changed. Remove the incomplete title with your title manager before retrying.".into());
      }
    }
    if !install {
      return Ok(HomeMenuStatus {
        phase: HomeMenuPhase::Ready,
        message: "Ready to add Wodex to the HOME Menu. The shortcut opens the NRO on your SD card."
          .into(),
      });
    }
    // Recheck immediately before mutation. An external installer may have run
    // while package preparation or the confirmation screen was open.
    if session.occupied()? {
      return Err("The HOME Menu identifier became occupied. Nothing was installed.".into());
    }
    install_package(&session, &package)?;
    Ok(HomeMenuStatus {
      phase: HomeMenuPhase::Installed,
      message: "Wodex was added to the HOME Menu. Keep sdmc:/switch/wodex/wodex.nro on the SD card; normal Wodex updates also update the shortcut's target.".into(),
    })
  }
}

fn read_exact(file: &mut File, mut buffer: &mut [u8]) -> Result<(), String> {
  while !buffer.is_empty() {
    let count = file.read(buffer)?;
    if count == 0 {
      return Err("The bundled shortcut or target NRO is truncated.".into());
    }
    buffer = &mut buffer[count..];
  }
  Ok(())
}

/// Only records created in this invocation are eligible for rollback. Metadata
/// is removed and committed before its content can be deleted. A failed cleanup
/// is surfaced explicitly, and referenced content is retained.
fn install_package(session: &Session, package: &Package) -> Result<(), String> {
  let api = &session.api;
  let handle = session.handle;
  checked(
    unsafe { (api.set_installing)(handle, true) },
    "Cannot protect shortcut installation from system exit",
  )?;
  let mut created = [false; 3];
  let mut placeholder = None;
  let mut metadata = false;
  let mut metadata_attempted = false;
  let mut registered = false;
  let mut registration_attempted = false;
  let mut content_registration_attempted = None;
  let result: Result<(), String> = (|| {
    for (index, (content, id)) in package.contents().zip(package.ids()).enumerate() {
      // A matching content ID is never ours to overwrite or clean up.
      if session.has_content(id)? {
        return Err(
          "Shortcut content became occupied. Existing content was left unchanged.".into(),
        );
      }
      let mut temporary = [0u8; 16];
      checked(
        unsafe { (api.placeholder_id)(handle, temporary.as_mut_ptr()) },
        "Cannot allocate an installation placeholder",
      )?;
      if session.has_placeholder(&temporary)? {
        return Err("The generated installation placeholder is already in use.".into());
      }
      placeholder = Some(temporary);
      checked(
        unsafe {
          (api.placeholder_create)(handle, id.as_ptr(), temporary.as_ptr(), content.len() as u64)
        },
        "Cannot create an installation placeholder",
      )?;
      for (chunk, data) in content.chunks(COPY_CHUNK).enumerate() {
        checked(
          unsafe {
            (api.placeholder_write)(
              handle,
              temporary.as_ptr(),
              (chunk * COPY_CHUNK) as u64,
              data.as_ptr(),
              data.len(),
            )
          },
          "Cannot write shortcut content",
        )?;
      }
      checked(unsafe { (api.placeholder_flush)(handle) }, "Cannot flush shortcut content")?;
      content_registration_attempted = Some(index);
      checked(
        unsafe { (api.placeholder_register)(handle, id.as_ptr(), temporary.as_ptr()) },
        "Cannot register shortcut content",
      )?;
      created[index] = true;
      placeholder = None;
      content_registration_attempted = None;
    }
    metadata_attempted = true;
    checked(
      unsafe {
        (api.metadata_set)(handle, &META_KEY, package.database().as_ptr(), package.database().len())
      },
      "Cannot write shortcut metadata",
    )?;
    metadata = true;
    checked(unsafe { (api.metadata_commit)(handle) }, "Cannot commit shortcut metadata")?;
    registration_attempted = true;
    checked(
      unsafe { (api.application_register)(handle, &META_KEY) },
      "Cannot add the HOME Menu entry",
    )?;
    registered = true;
    let confirmed = session.has_application().and_then(|present| {
      if present { session.has_metadata(5) } else { Ok(false) }
    }).map_err(|error| format!("{error} HOME Menu registration was requested. The shortcut content was retained; check the HOME Menu before retrying."))?;
    if !confirmed {
      return Err("Installation finished, but the HOME Menu did not confirm the entry. The installed content was retained; check the HOME Menu before retrying.".into());
    }
    Ok(())
  })();

  let mut cleanup_failed = false;
  if result.is_err() {
    if let (Some(index), Some(temporary)) = (content_registration_attempted, placeholder) {
      match (session.has_content(&package.ids()[index]), session.has_placeholder(&temporary)) {
        (Ok(true), Ok(false)) => {
          // Register consumed this invocation's placeholder despite an IPC
          // error. Only this confirmed transfer makes the content ours.
          created[index] = true;
          placeholder = None;
        }
        (Ok(_), Ok(_)) => (),
        _ => cleanup_failed = true,
      }
    }
  }
  let mut metadata_uncertain = false;
  if result.is_err() && metadata_attempted && !metadata {
    match session.has_metadata(5) {
      Ok(exists) => metadata = exists,
      Err(_) => {
        metadata_uncertain = true;
        cleanup_failed = true;
      }
    }
  }
  let mut registration_uncertain = false;
  if result.is_err() && registration_attempted && !registered {
    match session.has_application() {
      Ok(false) => (),
      Ok(true) | Err(_) => {
        // An IPC error may arrive after NS accepted the mutation. Never delete
        // content while a HOME Menu record might refer to it.
        registered = true;
        registration_uncertain = true;
      }
    }
  }
  if result.is_err() && !registered {
    if let Some(id) = placeholder {
      match session.has_placeholder(&id) {
        Ok(true) => {
          cleanup_failed |= unsafe { (api.placeholder_delete)(handle, id.as_ptr()) } != 0;
        }
        Ok(false) => (),
        Err(_) => cleanup_failed = true,
      }
    }
    let mut may_delete_content = !metadata_uncertain;
    if metadata {
      let removed = unsafe { (api.metadata_remove)(handle, &META_KEY) } == 0;
      let committed = unsafe { (api.metadata_commit)(handle) } == 0;
      may_delete_content = removed && committed;
      cleanup_failed |= !may_delete_content;
    }
    if may_delete_content {
      for (created, id) in created.into_iter().zip(package.ids()) {
        if created {
          cleanup_failed |= unsafe { (api.content_delete)(handle, id.as_ptr()) } != 0;
        }
      }
    }
  }
  let unlocked = unsafe { (api.set_installing)(handle, false) };
  if registration_uncertain {
    return Err(format!("{} The HOME Menu may already contain the shortcut. Its content was retained; check the HOME Menu before retrying.", result.unwrap_err()));
  }
  if cleanup_failed {
    return Err(format!("{} Cleanup could not finish. Existing title content was retained where needed; inspect the installation with your title manager before retrying.", result.unwrap_err()));
  }
  result?;
  checked(unlocked, "The shortcut was installed, but the system exit lock could not be released")
}

fn verify_bundle(data: &[u8]) -> Result<(), String> {
  use serde::Deserialize;
  use sha2::{Digest, Sha256};
  #[derive(Deserialize)]
  struct Manifest {
    format: String,
    title_id: String,
    target_nro: String,
    size: usize,
    sha256: String,
  }
  // This expected digest is compiled into executable code. A changed RomFS
  // manifest cannot authorize an altered loader, icon, target path or title.
  let manifest: Manifest =
    serde_json::from_str(include_str!("../../../romfs/home-menu/manifest.json"))
      .map_err(|_| String::from("The compiled HOME Menu manifest is invalid."))?;
  let hash = Sha256::digest(data);
  const HEX: &[u8; 16] = b"0123456789abcdef";
  let mut hex = [0u8; 64];
  for (index, byte) in hash.iter().enumerate() {
    hex[index * 2] = HEX[usize::from(byte >> 4)];
    hex[index * 2 + 1] = HEX[usize::from(byte & 15)];
  }
  if manifest.format != "WDXHOME1"
    || manifest.title_id != format!("{TITLE_ID:016x}")
    || manifest.target_nro != NRO_PATH
    || manifest.size != data.len()
    || manifest.sha256.as_bytes() != hex
  {
    return Err("The HOME Menu template does not match this Wodex build. Install a current official build before retrying.".into());
  }
  Ok(())
}
