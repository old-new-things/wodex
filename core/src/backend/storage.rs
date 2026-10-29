use alloc::ffi::CString;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::{c_char, CStr};
use core::sync::atomic::{AtomicU64, Ordering};

const MAX_SAVED_FILE_SIZE: usize = 32 * 1024 * 1024;
const MISSING: u32 = 1;
const EXISTS: u32 = 2;
const INTERRUPTED: u32 = 3;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Layout matches WodexFileInfo in wodex_fs.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileInfo {
  pub kind: u32,
  pub size: u64,
}

impl FileInfo {
  pub fn is_regular(&self) -> bool {
    self.kind == 1
  }

  pub fn is_directory(&self) -> bool {
    self.kind == 2
  }
}

/// Stateless native primitives only: flags, stat layout and errno are normalized
/// by the adapter so this code does not depend on a particular libc ABI.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FsApi {
  pub is_switch: bool,
  pub is_apple: bool,
  pub open: unsafe extern "C" fn(*const c_char, u32) -> i32,
  pub read: unsafe extern "C" fn(i32, *mut u8, usize) -> i64,
  pub write: unsafe extern "C" fn(i32, *const u8, usize) -> i64,
  pub seek: unsafe extern "C" fn(i32, u64) -> i32,
  pub close: unsafe extern "C" fn(i32) -> i32,
  pub sync: unsafe extern "C" fn(i32) -> i32,
  pub fstat: unsafe extern "C" fn(i32, *mut FileInfo) -> i32,
  pub stat: unsafe extern "C" fn(*const c_char, bool, *mut FileInfo) -> i32,
  pub rename: unsafe extern "C" fn(*const c_char, *const c_char) -> i32,
  pub unlink: unsafe extern "C" fn(*const c_char) -> i32,
  pub mkdir: unsafe extern "C" fn(*const c_char) -> i32,
  pub chmod: unsafe extern "C" fn(*const c_char, u32) -> i32,
  pub fchmod: unsafe extern "C" fn(i32, u32) -> i32,
  pub error_kind: unsafe extern "C" fn() -> u32,
  pub error_message: unsafe extern "C" fn() -> *const c_char,
  pub getenv: unsafe extern "C" fn(*const c_char) -> *const c_char,
  pub pid: unsafe extern "C" fn() -> u64,
}

#[derive(Clone, Copy)]
pub struct Fs {
  api: FsApi,
}

struct IoError {
  kind: u32,
  message: String,
}

impl IoError {
  fn context(self, operation: &str) -> String {
    format!("{operation}: {}", self.message)
  }
}

impl Fs {
  /// # Safety
  /// Every callback must remain callable for the lifetime of any copied Fs or
  /// File. They must obey wodex_fs.h, never unwind, and expose the last error on
  /// the calling thread until the next native operation on that thread.
  pub unsafe fn new(api: &FsApi) -> Self {
    Self { api: *api }
  }

  pub fn is_switch(&self) -> bool {
    self.api.is_switch
  }

  pub fn is_apple(&self) -> bool {
    self.api.is_apple
  }

  fn error(&self) -> IoError {
    unsafe {
      let kind = (self.api.error_kind)();
      let message = (self.api.error_message)();
      IoError {
        kind,
        message: if message.is_null() {
          "Unknown filesystem error".into()
        } else {
          CStr::from_ptr(message).to_string_lossy().into_owned()
        },
      }
    }
  }

  fn retry(&self, mut operation: impl FnMut() -> i32) -> Result<i32, IoError> {
    loop {
      let value = operation();
      if value >= 0 {
        return Ok(value);
      }
      let error = self.error();
      if error.kind != INTERRUPTED {
        return Err(error);
      }
    }
  }

  pub fn environment(&self, name: &str) -> String {
    let Ok(name) = CString::new(name) else {
      return String::new();
    };
    unsafe {
      let value = (self.api.getenv)(name.as_ptr());
      if value.is_null() {
        String::new()
      } else {
        CStr::from_ptr(value).to_string_lossy().into_owned()
      }
    }
  }

  pub fn data_dir(&self) -> Result<String, String> {
    let path = if self.is_switch() {
      "sdmc:/switch/wodex".into()
    } else {
      let override_path = self.environment("WODEX_DATA_DIR");
      if !override_path.is_empty() {
        override_path
      } else {
        let home = self.environment("HOME");
        if self.is_apple() {
          if home.is_empty() {
            return Err("Set HOME or WODEX_DATA_DIR to save conversations.".into());
          }
          format!("{home}/Library/Application Support/Wodex")
        } else {
          let xdg = self.environment("XDG_DATA_HOME");
          if xdg.starts_with('/') {
            format!("{xdg}/wodex")
          } else if !home.is_empty() {
            format!("{home}/.local/share/wodex")
          } else {
            return Err("Set HOME or WODEX_DATA_DIR to save conversations.".into());
          }
        }
      }
    };
    self.create_directories(&path)?;
    Ok(path)
  }

  pub fn font_path(&self, resource_dir: &str) -> Result<String, String> {
    const BUNDLED_FONT: &str = "/fonts/NotoSansCJKsc-Regular.otf";
    if self.is_switch() {
      let path = format!("romfs:{BUNDLED_FONT}");
      // Empty requests the native shared-font service. Filesystem selection
      // remains independent of the libnx language/font API.
      return Ok(if self.readable_font(&path) { path } else { String::new() });
    }
    let configured = self.environment("WODEX_FONT");
    if !configured.is_empty() {
      if !self.readable_font(&configured) {
        return Err("WODEX_FONT does not point to a readable font.".into());
      }
      return Ok(configured);
    }
    if !resource_dir.is_empty() {
      let bundled = format!("{resource_dir}{BUNDLED_FONT}");
      if self.readable_font(&bundled) {
        return Ok(bundled);
      }
    }
    for candidate in [
      "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
      "/Library/Fonts/Arial Unicode.ttf",
      "/System/Library/Fonts/Supplemental/Arial.ttf",
      "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
      "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
      "/usr/share/fonts/TTF/DejaVuSans.ttf",
      "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    ] {
      if self.readable_font(candidate) {
        return Ok(candidate.into());
      }
    }
    Err("No font found. Set WODEX_FONT to a TrueType or OpenType font.".into())
  }

  pub fn monospace_font_path(&self, resource_dir: &str) -> Result<String, String> {
    const BUNDLED_FONT: &str = "/fonts/SourceCodePro-Regular.otf";
    let bundled = if self.is_switch() {
      format!("romfs:{BUNDLED_FONT}")
    } else if !resource_dir.is_empty() {
      format!("{resource_dir}{BUNDLED_FONT}")
    } else {
      String::new()
    };
    if !bundled.is_empty() && self.readable_font(&bundled) {
      return Ok(bundled);
    }
    // Recover from missing optional code assets using the same desktop or
    // Switch shared-font fallback as ordinary text.
    self.font_path(resource_dir)
  }

  fn readable_font(&self, path: &str) -> bool {
    if !matches!(self.stat(path, true), Ok(Some(info)) if info.is_regular()) {
      return false;
    }
    // Font installations commonly use symlinks. This explicitly follows them;
    // save/update reads retain the stricter no-follow open mode.
    self.open(path, 3).is_ok_and(|file| file.info().is_ok_and(|info| info.is_regular()))
  }

  fn open(&self, path: &str, mode: u32) -> Result<File, IoError> {
    let path = path_cstring(path).map_err(|message| IoError { kind: 4, message })?;
    let descriptor = self.retry(|| unsafe { (self.api.open)(path.as_ptr(), mode) })?;
    Ok(File { fs: *self, descriptor })
  }

  fn open_regular(&self, path: &str, mode: u32) -> Result<File, String> {
    // This also rejects symlinks on native libraries lacking O_NOFOLLOW. The
    // descriptor check below rejects directories/devices and concurrent changes.
    if let Some(info) = self.stat(path, false)? {
      if !info.is_regular() {
        return Err("The file is not a regular file.".into());
      }
    }
    let file = self.open(path, mode).map_err(|error| error.context("Cannot open file"))?;
    if !file.info()?.is_regular() {
      return Err("The file is not a regular file.".into());
    }
    Ok(file)
  }

  pub fn open_read(&self, path: &str) -> Result<File, String> {
    self.open_regular(path, 0)
  }

  /// Open an existing regular file without creating, truncating or appending.
  /// Both reads and writes begin at offset zero; callers seek before resuming.
  pub fn open_read_write(&self, path: &str) -> Result<File, String> {
    self.open_regular(path, 4)
  }

  pub fn create_new(&self, path: &str) -> Result<File, String> {
    self.create_exclusive(path).map_err(|error| error.context("Cannot create file"))
  }

  fn create_exclusive(&self, path: &str) -> Result<File, IoError> {
    let file = self.open(path, 1)?;
    if !self.is_switch() {
      if let Err(error) = self.retry(|| unsafe { (self.api.fchmod)(file.descriptor, 0o600) }) {
        drop(file);
        let _ = self.remove(path, true);
        return Err(error);
      }
    }
    Ok(file)
  }

  pub fn stat(&self, path: &str, follow: bool) -> Result<Option<FileInfo>, String> {
    let path = path_cstring(path)?;
    let mut output = FileInfo::default();
    match self.retry(|| unsafe { (self.api.stat)(path.as_ptr(), follow, &mut output) }) {
      Ok(_) => Ok(Some(output)),
      Err(error) if error.kind == MISSING => Ok(None),
      Err(error) => Err(error.context("Cannot inspect file")),
    }
  }

  fn rename_raw(&self, from: &str, to: &str) -> Result<(), IoError> {
    let from = path_cstring(from).map_err(|message| IoError { kind: 4, message })?;
    let to = path_cstring(to).map_err(|message| IoError { kind: 4, message })?;
    self.retry(|| unsafe { (self.api.rename)(from.as_ptr(), to.as_ptr()) })?;
    Ok(())
  }

  pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
    self.rename_raw(from, to).map_err(|error| error.context("Cannot rename file"))
  }

  pub fn remove(&self, path: &str, missing_ok: bool) -> Result<(), String> {
    let path = path_cstring(path)?;
    match self.retry(|| unsafe { (self.api.unlink)(path.as_ptr()) }) {
      Ok(_) => Ok(()),
      Err(error) if missing_ok && error.kind == MISSING => Ok(()),
      Err(error) => Err(error.context("Cannot remove file")),
    }
  }

  pub fn sync_parent(&self, path: &str) -> Result<(), String> {
    // libnx fsync flushes the SD file. Directory fsync is unsupported, and SD
    // filesystems have no savedata commit step.
    if self.is_switch() {
      return Ok(());
    }
    let directory = self
      .open(parent_directory(path), 2)
      .map_err(|error| error.context("Cannot open save directory"))?;
    directory.sync().map_err(|error| format!("Cannot flush save directory: {error}"))
  }

  pub fn create_directories(&self, path: &str) -> Result<(), String> {
    let _ = path_cstring(path)?;
    let start = if self.is_switch() {
      path.find(":/").map(|index| index + 2).unwrap_or(0)
    } else {
      usize::from(path.starts_with('/'))
    };
    for end in start..=path.len() {
      if end != path.len() && path.as_bytes()[end] != b'/' {
        continue;
      }
      let directory = &path[..end];
      if directory.is_empty() || directory.ends_with(":/") {
        continue;
      }
      let directory_c = path_cstring(directory)?;
      match self.retry(|| unsafe { (self.api.mkdir)(directory_c.as_ptr()) }) {
        Ok(_) => {}
        Err(error) if error.kind == EXISTS => {}
        Err(error) => return Err(error.context("Cannot create data directory")),
      }
      if !self.stat(directory, true)?.is_some_and(|info| info.is_directory()) {
        return Err("The data path is not a directory.".into());
      }
    }
    if !self.is_switch() {
      let path = path_cstring(path)?;
      // Protect only the application's final directory, never shared parents.
      self
        .retry(|| unsafe { (self.api.chmod)(path.as_ptr(), 0o700) })
        .map_err(|error| error.context("Cannot protect data directory"))?;
    }
    Ok(())
  }

  pub fn load_file(&self, path: &str) -> Result<Vec<u8>, String> {
    let _ = path_cstring(path)?;
    let source = if self.stat(path, false)?.is_some() {
      path.to_string()
    } else if self.is_switch() {
      let backup = format!("{path}.bak");
      if self.stat(&backup, false)?.is_none() {
        return Ok(Vec::new());
      }
      backup
    } else {
      return Ok(Vec::new());
    };
    let mut file = self.open_read(&source)?;
    let size = file.size()?;
    if size > MAX_SAVED_FILE_SIZE as u64 {
      return Err("Saved data exceeds the 32 MiB size limit.".into());
    }
    let mut content = Vec::with_capacity(size as usize);
    let mut buffer = [0; 16384];
    loop {
      let count = file.read(&mut buffer)?;
      if count == 0 {
        return Ok(content);
      }
      if content.len() + count > MAX_SAVED_FILE_SIZE {
        return Err("Saved data exceeds the 32 MiB size limit.".into());
      }
      content.extend_from_slice(&buffer[..count]);
    }
  }

  pub fn atomic_write(&self, path: &str, content: &[u8]) -> Result<(), String> {
    validate_saved_input(path, content)?;
    // Exclusive creation preserves a stale temp file from another instance.
    let (temporary, mut file) = self.temporary_file(path)?;
    let result = (|| {
      file.write_all(content)?;
      file.sync()?;
      file.close()?;
      match self.rename_raw(&temporary, path) {
        Ok(()) => {}
        Err(error) if self.is_switch() && error.kind == EXISTS => {
          let backup = format!("{path}.bak");
          self.remove(&backup, true)?;
          self.rename(path, &backup)?;
          if let Err(error) = self.rename_raw(&temporary, path) {
            let message = error.context("Cannot replace saved data");
            if self.rename(&backup, path).is_err() {
              return Err(format!("{message}. The previous save remains at {backup}."));
            }
            return Err(message);
          }
        }
        Err(error) => return Err(error.context("Cannot replace saved data")),
      }
      self.sync_parent(path)
    })();
    if result.is_err() {
      let _ = self.remove(&temporary, true);
    }
    result
  }

  fn temporary_file(&self, path: &str) -> Result<(String, File), String> {
    for _ in 0..128 {
      let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
      let pid = unsafe { (self.api.pid)() };
      let temporary = format!("{path}.tmp.{pid}.{counter}");
      match self.create_exclusive(&temporary) {
        Ok(file) => return Ok((temporary, file)),
        Err(error) if error.kind == EXISTS => {}
        Err(error) => return Err(error.context("Cannot create temporary save file")),
      }
    }
    Err("Cannot create a unique temporary save file.".into())
  }

  pub fn preserve_file(&self, path: &str, content: &[u8]) -> Result<String, String> {
    validate_saved_input(path, content)?;
    for index in 1..=9999 {
      let recovery = format!("{path}.recovery-{index}");
      let mut file = match self.create_exclusive(&recovery) {
        Ok(file) => file,
        Err(error) if error.kind == EXISTS => continue,
        Err(error) => return Err(error.context("Cannot create recovery file")),
      };
      let result = (|| {
        file.write_all(content)?;
        file.sync()?;
        file.close()?;
        self.sync_parent(&recovery)
      })();
      if let Err(error) = result {
        let _ = self.remove(&recovery, true);
        return Err(error);
      }
      return Ok(recovery);
    }
    Err(
      "The recovery file limit was reached. Move older recovery files before trying again.".into(),
    )
  }
}

pub struct File {
  fs: Fs,
  descriptor: i32,
}

impl File {
  pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
    loop {
      let count = unsafe { (self.fs.api.read)(self.descriptor, buffer.as_mut_ptr(), buffer.len()) };
      if count >= 0 {
        return checked_count(count, buffer.len());
      }
      let error = self.fs.error();
      if error.kind != INTERRUPTED {
        return Err(error.context("Cannot read file"));
      }
    }
  }

  pub fn write_all(&mut self, mut content: &[u8]) -> Result<(), String> {
    while !content.is_empty() {
      let count = unsafe { (self.fs.api.write)(self.descriptor, content.as_ptr(), content.len()) };
      if count < 0 {
        let error = self.fs.error();
        if error.kind == INTERRUPTED {
          continue;
        }
        return Err(error.context("Cannot write file"));
      }
      let count = checked_count(count, content.len())?;
      if count == 0 {
        return Err("Cannot finish writing file.".into());
      }
      content = &content[count..];
    }
    Ok(())
  }

  pub fn seek(&mut self, offset: u64) -> Result<(), String> {
    self
      .fs
      .retry(|| unsafe { (self.fs.api.seek)(self.descriptor, offset) })
      .map_err(|error| error.context("Cannot seek file"))?;
    Ok(())
  }

  pub fn sync(&self) -> Result<(), String> {
    self
      .fs
      .retry(|| unsafe { (self.fs.api.sync)(self.descriptor) })
      .map_err(|error| error.context("Cannot flush file"))?;
    Ok(())
  }

  pub fn info(&self) -> Result<FileInfo, String> {
    let mut info = FileInfo::default();
    self
      .fs
      .retry(|| unsafe { (self.fs.api.fstat)(self.descriptor, &mut info) })
      .map_err(|error| error.context("Cannot inspect file"))?;
    Ok(info)
  }

  pub fn size(&self) -> Result<u64, String> {
    Ok(self.info()?.size)
  }

  pub fn close(mut self) -> Result<(), String> {
    let descriptor = core::mem::replace(&mut self.descriptor, -1);
    // Never retry close on EINTR: some libc implementations already released
    // the descriptor and a retry could close another thread's reused handle.
    if unsafe { (self.fs.api.close)(descriptor) } == 0 {
      Ok(())
    } else {
      Err(self.fs.error().context("Cannot close file"))
    }
  }
}

impl Drop for File {
  fn drop(&mut self) {
    if self.descriptor >= 0 {
      unsafe {
        (self.fs.api.close)(self.descriptor);
      }
    }
  }
}

fn path_cstring(path: &str) -> Result<CString, String> {
  if path.is_empty() {
    return Err("The file path is empty.".into());
  }
  CString::new(path).map_err(|_| "The file path contains a NUL byte.".into())
}

fn parent_directory(path: &str) -> &str {
  match path.rfind('/') {
    Some(0) => "/",
    Some(index) => &path[..index],
    None => ".",
  }
}

fn validate_saved_input(path: &str, content: &[u8]) -> Result<(), String> {
  let _ = path_cstring(path)?;
  if content.len() > MAX_SAVED_FILE_SIZE {
    return Err("Saved data exceeds the 32 MiB size limit.".into());
  }
  Ok(())
}

fn checked_count(count: i64, maximum: usize) -> Result<usize, String> {
  if count < 0 || count as u64 > maximum as u64 {
    Err("The native filesystem returned an invalid byte count.".into())
  } else {
    Ok(count as usize)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::cell::RefCell;
  use std::collections::BTreeMap;
  use std::fs::{self, OpenOptions};
  use std::io::{Read, Seek, SeekFrom, Write};
  use std::path::PathBuf;

  #[derive(Default)]
  struct TestIo {
    files: BTreeMap<i32, std::fs::File>,
    next_fd: i32,
    error: u32,
    switch: bool,
    short_write: bool,
    interrupt_write: bool,
    interrupt_read: bool,
    fail_sync: bool,
    fail_replace: bool,
    fail_rollback: bool,
    fail_permissions: bool,
    fstat_kind: Option<u32>,
    environment: BTreeMap<String, CString>,
  }

  std::thread_local! {
    static IO: RefCell<TestIo> = RefCell::new(TestIo::default());
  }

  fn outcome<T>(io: &mut TestIo, value: std::io::Result<T>) -> Option<T> {
    match value {
      Ok(value) => Some(value),
      Err(error) => {
        io.error = match error.kind() {
          std::io::ErrorKind::NotFound => MISSING,
          std::io::ErrorKind::AlreadyExists => EXISTS,
          std::io::ErrorKind::Interrupted => INTERRUPTED,
          _ => 4,
        };
        None
      }
    }
  }

  unsafe fn text<'a>(value: *const c_char) -> &'a str {
    CStr::from_ptr(value).to_str().unwrap()
  }

  unsafe extern "C" fn open(path: *const c_char, mode: u32) -> i32 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      let mut options = OpenOptions::new();
      match mode {
        0 | 2 | 3 => {
          options.read(true);
        }
        1 => {
          options.write(true).create_new(true);
        }
        4 => {
          options.read(true).write(true);
        }
        _ => {
          io.error = 4;
          return -1;
        }
      }
      let Some(file) = outcome(&mut io, options.open(text(path))) else { return -1 };
      io.next_fd += 1;
      let fd = io.next_fd;
      io.files.insert(fd, file);
      fd
    })
  }

  unsafe extern "C" fn read(fd: i32, buffer: *mut u8, len: usize) -> i64 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      if core::mem::take(&mut io.interrupt_read) {
        io.error = INTERRUPTED;
        return -1;
      }
      let result =
        io.files.get_mut(&fd).unwrap().read(core::slice::from_raw_parts_mut(buffer, len));
      outcome(&mut io, result).map_or(-1, |count| count as i64)
    })
  }

  unsafe extern "C" fn write(fd: i32, buffer: *const u8, len: usize) -> i64 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      if core::mem::take(&mut io.interrupt_write) {
        io.error = INTERRUPTED;
        return -1;
      }
      let size = if io.short_write { len.min(3) } else { len };
      let result = io.files.get_mut(&fd).unwrap().write(core::slice::from_raw_parts(buffer, size));
      outcome(&mut io, result).map_or(-1, |count| count as i64)
    })
  }

  unsafe extern "C" fn seek(fd: i32, offset: u64) -> i32 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      let result = io.files.get_mut(&fd).unwrap().seek(SeekFrom::Start(offset));
      outcome(&mut io, result).map_or(-1, |_| 0)
    })
  }

  unsafe extern "C" fn close(fd: i32) -> i32 {
    IO.with(|state| {
      state.borrow_mut().files.remove(&fd);
    });
    0
  }

  unsafe extern "C" fn sync(fd: i32) -> i32 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      if io.fail_sync {
        io.error = 4;
        return -1;
      }
      let result = io.files.get(&fd).unwrap().sync_all();
      outcome(&mut io, result).map_or(-1, |_| 0)
    })
  }

  fn metadata(value: std::fs::Metadata) -> FileInfo {
    FileInfo {
      kind: if value.is_file() {
        1
      } else if value.is_dir() {
        2
      } else if value.file_type().is_symlink() {
        3
      } else {
        4
      },
      size: value.len(),
    }
  }

  unsafe extern "C" fn fstat(fd: i32, output: *mut FileInfo) -> i32 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      let result = io.files.get(&fd).unwrap().metadata();
      let Some(value) = outcome(&mut io, result) else { return -1 };
      *output = metadata(value);
      if let Some(kind) = io.fstat_kind {
        (*output).kind = kind;
      }
      0
    })
  }

  unsafe extern "C" fn stat(path: *const c_char, follow: bool, output: *mut FileInfo) -> i32 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      let result = if follow { fs::metadata(text(path)) } else { fs::symlink_metadata(text(path)) };
      let Some(value) = outcome(&mut io, result) else { return -1 };
      *output = metadata(value);
      0
    })
  }

  unsafe extern "C" fn rename(from: *const c_char, to: *const c_char) -> i32 {
    let from = text(from);
    let to = text(to);
    IO.with(|state| {
      let mut io = state.borrow_mut();
      if io.switch && fs::symlink_metadata(to).is_ok() {
        io.error = EXISTS;
        return -1;
      }
      if from.contains(".tmp.") && !from.ends_with(".bak") && core::mem::take(&mut io.fail_replace)
      {
        io.error = 4;
        return -1;
      }
      if from.ends_with(".bak") && io.fail_rollback {
        io.error = 4;
        return -1;
      }
      outcome(&mut io, fs::rename(from, to)).map_or(-1, |_| 0)
    })
  }

  unsafe extern "C" fn unlink(path: *const c_char) -> i32 {
    IO.with(|state| outcome(&mut state.borrow_mut(), fs::remove_file(text(path))).map_or(-1, |_| 0))
  }

  unsafe extern "C" fn mkdir(path: *const c_char) -> i32 {
    IO.with(|state| outcome(&mut state.borrow_mut(), fs::create_dir(text(path))).map_or(-1, |_| 0))
  }

  unsafe extern "C" fn chmod(_path: *const c_char, _mode: u32) -> i32 {
    0
  }

  unsafe extern "C" fn fchmod(_fd: i32, _mode: u32) -> i32 {
    IO.with(|state| {
      let mut io = state.borrow_mut();
      if io.fail_permissions {
        io.error = 4;
        -1
      } else {
        0
      }
    })
  }

  unsafe extern "C" fn error_kind() -> u32 {
    IO.with(|state| state.borrow().error)
  }
  unsafe extern "C" fn error_message() -> *const c_char {
    c"injected filesystem failure".as_ptr()
  }

  unsafe extern "C" fn getenv(name: *const c_char) -> *const c_char {
    IO.with(|state| {
      state.borrow().environment.get(text(name)).map_or(core::ptr::null(), |s| s.as_ptr())
    })
  }

  unsafe extern "C" fn pid() -> u64 {
    std::process::id() as u64
  }

  struct Fixture {
    fs: Fs,
    directory: PathBuf,
  }

  impl Fixture {
    fn new(switch: bool) -> Self {
      IO.with(|state| *state.borrow_mut() = TestIo { switch, ..TestIo::default() });
      let directory = std::env::temp_dir().join(format!(
        "wodex-rust-storage-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
      ));
      fs::create_dir(&directory).unwrap();
      let api = FsApi {
        is_switch: switch,
        is_apple: false,
        open,
        read,
        write,
        seek,
        close,
        sync,
        fstat,
        stat,
        rename,
        unlink,
        mkdir,
        chmod,
        fchmod,
        error_kind,
        error_message,
        getenv,
        pid,
      };
      Self { fs: unsafe { Fs::new(&api) }, directory }
    }

    fn path(&self) -> String {
      self.directory.join("state.json").to_str().unwrap().to_string()
    }
  }

  impl Drop for Fixture {
    fn drop(&mut self) {
      IO.with(|state| assert!(state.borrow().files.is_empty(), "A file descriptor leaked"));
      fs::remove_dir_all(&self.directory).unwrap();
    }
  }

  #[test]
  fn durable_write_retries_interruptions_and_short_writes_without_altering_bytes() {
    let fixture = Fixture::new(false);
    IO.with(|state| {
      let mut io = state.borrow_mut();
      io.short_write = true;
      io.interrupt_write = true;
      io.interrupt_read = true;
    });
    let content = b"{\x00\xff invalid UTF-8 Chinese \xe4\xb8\xad\xe6\x96\x87}";
    fixture.fs.atomic_write(&fixture.path(), content).unwrap();
    assert_eq!(fixture.fs.load_file(&fixture.path()).unwrap(), content);
    let mut file = fixture.fs.open_read(&fixture.path()).unwrap();
    file.seek(3).unwrap();
    let mut buffer = [0; 2];
    assert_eq!(file.read(&mut buffer).unwrap(), 2);
    assert_eq!(&buffer, &content[3..5]);
    file.close().unwrap();
  }

  #[test]
  fn failed_flush_keeps_previous_save_and_removes_partial_file() {
    let fixture = Fixture::new(false);
    fixture.fs.atomic_write(&fixture.path(), b"previous").unwrap();
    IO.with(|state| state.borrow_mut().fail_sync = true);
    assert!(fixture.fs.atomic_write(&fixture.path(), b"replacement").is_err());
    assert_eq!(fixture.fs.load_file(&fixture.path()).unwrap(), b"previous");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 1);
  }

  #[test]
  fn switch_rotation_restores_save_when_replacement_fails() {
    let fixture = Fixture::new(true);
    fixture.fs.atomic_write(&fixture.path(), b"previous").unwrap();
    IO.with(|state| state.borrow_mut().fail_replace = true);
    assert!(fixture.fs.atomic_write(&fixture.path(), b"replacement").is_err());
    assert_eq!(fixture.fs.load_file(&fixture.path()).unwrap(), b"previous");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 1);
    fixture.fs.atomic_write(&fixture.path(), b"successful replacement").unwrap();
    assert_eq!(fixture.fs.load_file(&fixture.path()).unwrap(), b"successful replacement");
    assert_eq!(fixture.fs.load_file(&format!("{}.bak", fixture.path())).unwrap(), b"previous");
  }

  #[test]
  fn switch_failed_rollback_preserves_recoverable_backup() {
    let fixture = Fixture::new(true);
    fixture.fs.atomic_write(&fixture.path(), b"previous").unwrap();
    IO.with(|state| {
      let mut io = state.borrow_mut();
      io.fail_replace = true;
      io.fail_rollback = true;
    });
    let error = fixture.fs.atomic_write(&fixture.path(), b"replacement").unwrap_err();
    assert!(error.contains(".bak"));
    assert!(fixture.fs.stat(&fixture.path(), false).unwrap().is_none());
    assert_eq!(fixture.fs.load_file(&fixture.path()).unwrap(), b"previous");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 1);
  }

  #[test]
  fn recovery_is_exclusive_and_permission_failure_leaves_no_partial_copy() {
    let fixture = Fixture::new(false);
    let first = fixture.fs.preserve_file(&fixture.path(), b"first\0\xff").unwrap();
    let second = fixture.fs.preserve_file(&fixture.path(), b"second").unwrap();
    assert!(first.ends_with(".recovery-1"));
    assert!(second.ends_with(".recovery-2"));
    assert_eq!(fixture.fs.load_file(&first).unwrap(), b"first\0\xff");
    IO.with(|state| state.borrow_mut().fail_permissions = true);
    assert!(fixture.fs.preserve_file(&fixture.path(), b"third").is_err());
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 2);
  }

  #[test]
  fn path_and_file_kind_guards_apply_to_reads_and_writes() {
    let fixture = Fixture::new(false);
    assert!(fixture.fs.load_file("missing\0suffix").is_err());
    assert!(fixture.fs.atomic_write("", b"data").is_err());
    assert!(fixture.fs.load_file(fixture.directory.to_str().unwrap()).is_err());
    assert_eq!(fixture.fs.load_file(&fixture.path()).unwrap(), b"");
    #[cfg(unix)]
    {
      let target = fixture.directory.join("target");
      fs::write(&target, b"must not follow").unwrap();
      std::os::unix::fs::symlink(&target, fixture.path()).unwrap();
      assert!(fixture.fs.open_read(&fixture.path()).is_err());
      assert!(fixture.fs.open_read_write(&fixture.path()).is_err());
      assert!(fixture.fs.create_new(&fixture.path()).is_err());
      assert_eq!(fs::read(&target).unwrap(), b"must not follow");
    }
  }

  #[test]
  fn read_write_open_preserves_bytes_and_supports_explicit_resume_offsets() {
    for switch in [false, true] {
      let fixture = Fixture::new(switch);
      fs::write(fixture.path(), b"abcdefgh").unwrap();
      let mut file = fixture.fs.open_read_write(&fixture.path()).unwrap();
      assert_eq!(file.size().unwrap(), 8);
      let mut prefix = [0; 3];
      assert_eq!(file.read(&mut prefix).unwrap(), 3);
      assert_eq!(&prefix, b"abc");
      file.seek(8).unwrap();
      file.write_all(b"ij").unwrap();
      // A write after seeking to zero must replace bytes, not append them.
      file.seek(0).unwrap();
      file.write_all(b"A").unwrap();
      file.sync().unwrap();
      file.close().unwrap();
      assert_eq!(fs::read(fixture.path()).unwrap(), b"Abcdefghij");
    }
  }

  #[test]
  fn read_write_open_requires_an_existing_regular_file_and_closes_rejected_handles() {
    let fixture = Fixture::new(false);
    assert!(fixture.fs.open_read_write(&fixture.path()).is_err());
    assert!(!fixture.directory.join("state.json").exists());
    assert!(fixture.fs.open_read_write(fixture.directory.to_str().unwrap()).is_err());
    fs::write(fixture.path(), b"keep existing content").unwrap();
    IO.with(|state| state.borrow_mut().fstat_kind = Some(2));
    assert!(fixture.fs.open_read_write(&fixture.path()).is_err());
    IO.with(|state| assert!(state.borrow().files.is_empty()));
    assert_eq!(fs::read(fixture.path()).unwrap(), b"keep existing content");
  }

  #[test]
  fn data_directory_resolution_rejects_relative_xdg_and_honors_override() {
    let fixture = Fixture::new(false);
    IO.with(|state| {
      let mut io = state.borrow_mut();
      io.environment
        .insert("HOME".into(), CString::new(fixture.directory.to_str().unwrap()).unwrap());
      io.environment.insert("XDG_DATA_HOME".into(), c"relative/data".to_owned());
    });
    let directory = fixture.fs.data_dir().unwrap();
    assert!(directory.ends_with("/.local/share/wodex"));
    let override_path = fixture.directory.join("custom");
    IO.with(|state| {
      state
        .borrow_mut()
        .environment
        .insert("WODEX_DATA_DIR".into(), CString::new(override_path.to_str().unwrap()).unwrap());
    });
    assert_eq!(fixture.fs.data_dir().unwrap(), override_path.to_str().unwrap());
    let switch_fs = Fs { api: FsApi { is_switch: true, ..fixture.fs.api } };
    assert_eq!(switch_fs.environment("HOME"), fixture.directory.to_str().unwrap());
  }

  #[test]
  fn font_selection_prefers_explicit_paths_and_preserves_symlink_support() {
    let fixture = Fixture::new(false);
    let directory = fixture.directory.join("fonts");
    fs::create_dir(&directory).unwrap();
    let bundled = directory.join("NotoSansCJKsc-Regular.otf");
    fs::write(&bundled, b"test font").unwrap();
    assert_eq!(
      fixture.fs.font_path(fixture.directory.to_str().unwrap()).unwrap(),
      bundled.to_str().unwrap()
    );
    assert_eq!(
      fixture.fs.monospace_font_path(fixture.directory.to_str().unwrap()).unwrap(),
      bundled.to_str().unwrap()
    );
    let monospace = directory.join("SourceCodePro-Regular.otf");
    fs::write(&monospace, b"test code font").unwrap();
    assert_eq!(
      fixture.fs.monospace_font_path(fixture.directory.to_str().unwrap()).unwrap(),
      monospace.to_str().unwrap()
    );
    let override_path = fixture.directory.join("override.otf");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&bundled, &override_path).unwrap();
    #[cfg(not(unix))]
    fs::write(&override_path, b"override font").unwrap();
    IO.with(|state| {
      state
        .borrow_mut()
        .environment
        .insert("WODEX_FONT".into(), CString::new(override_path.to_str().unwrap()).unwrap());
    });
    assert_eq!(
      fixture.fs.font_path(fixture.directory.to_str().unwrap()).unwrap(),
      override_path.to_str().unwrap()
    );
    assert_eq!(
      fixture.fs.monospace_font_path(fixture.directory.to_str().unwrap()).unwrap(),
      monospace.to_str().unwrap()
    );
    fs::remove_file(&monospace).unwrap();
    assert_eq!(
      fixture.fs.monospace_font_path(fixture.directory.to_str().unwrap()).unwrap(),
      override_path.to_str().unwrap()
    );
    // Font reads intentionally follow the symlink, while saved data and NROs
    // retain no-follow validation.
    #[cfg(unix)]
    assert!(fixture.fs.open_read(override_path.to_str().unwrap()).is_err());
    IO.with(|state| {
      state
        .borrow_mut()
        .environment
        .insert("WODEX_FONT".into(), CString::new(directory.to_str().unwrap()).unwrap());
    });
    assert!(fixture
      .fs
      .font_path(fixture.directory.to_str().unwrap())
      .unwrap_err()
      .contains("WODEX_FONT"));
  }
}
