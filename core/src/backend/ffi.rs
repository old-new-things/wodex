//! C entry points used by process startup and native regression harnesses.
use super::{
  http::Http,
  runtime::Runtime,
  storage::{Fs, FsApi},
  transport::Transport,
  updater::{self, Updater},
  BackendApi, PendingHandle,
};
use crate::app::host::TransportEvent;
use alloc::{
  boxed::Box,
  collections::VecDeque,
  ffi::CString,
  string::{String, ToString},
  vec::Vec,
};
use core::ffi::{c_char, CStr};

pub struct Buffer(Vec<u8>);
pub struct TransportBridge {
  value: Transport,
  queued: VecDeque<TransportEvent>,
  current: Option<TransportEvent>,
  error: CString,
}

#[repr(C)]
pub struct NativeEvent {
  kind: u32,
  data: *const u8,
  length: usize,
  http_status: u16,
  error: *const c_char,
}

unsafe fn text<'a>(value: *const c_char) -> Result<&'a str, String> {
  if value.is_null() {
    return Err("Missing native text argument.".into());
  }
  CStr::from_ptr(value).to_str().map_err(|_| "Invalid UTF-8 argument.".into())
}
unsafe fn bytes<'a>(data: *const u8, len: usize) -> Result<&'a [u8], String> {
  if len == 0 {
    Ok(&[])
  } else if data.is_null() {
    Err("Invalid native data buffer.".into())
  } else {
    Ok(core::slice::from_raw_parts(data, len))
  }
}
fn owned(value: &str) -> *mut c_char {
  CString::new(value.replace('\0', " ")).map_or(core::ptr::null_mut(), CString::into_raw)
}
unsafe fn output_error(output: *mut *mut c_char, error: Option<&str>) {
  if let Some(output) = output.as_mut() {
    *output = error.map_or(core::ptr::null_mut(), owned);
  }
}
unsafe fn checked<T>(result: Result<T, String>, error: *mut *mut c_char) -> Option<T> {
  match result {
    Ok(value) => {
      output_error(error, None);
      Some(value)
    }
    Err(message) => {
      output_error(error, Some(&message));
      None
    }
  }
}
unsafe fn filesystem(api: *const FsApi) -> Result<Fs, String> {
  api.as_ref().map(|api| Fs::new(api)).ok_or_else(|| "Missing native filesystem bindings.".into())
}
unsafe fn services(api: *const BackendApi) -> Result<(Runtime, Http, Fs), String> {
  let api = api.as_ref().ok_or("Missing native backend bindings.")?;
  Ok((Runtime::new(&api.runtime), Http::new(&api.http)?, Fs::new(&api.fs)))
}

#[no_mangle]
pub unsafe extern "C" fn wodex_buffer_data(buffer: *const Buffer) -> *const u8 {
  buffer.as_ref().map_or(core::ptr::null(), |buffer| buffer.0.as_ptr())
}
#[no_mangle]
pub unsafe extern "C" fn wodex_buffer_len(buffer: *const Buffer) -> usize {
  buffer.as_ref().map_or(0, |buffer| buffer.0.len())
}
#[no_mangle]
pub unsafe extern "C" fn wodex_buffer_free(buffer: *mut Buffer) {
  if !buffer.is_null() {
    drop(Box::from_raw(buffer));
  }
}
#[no_mangle]
pub unsafe extern "C" fn wodex_storage_data_dir(
  api: *const FsApi,
  error: *mut *mut c_char,
) -> *mut c_char {
  checked(filesystem(api).and_then(|fs| fs.data_dir()), error)
    .map_or(core::ptr::null_mut(), |path| owned(&path))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_storage_font_path(
  api: *const FsApi,
  resources: *const c_char,
  error: *mut *mut c_char,
) -> *mut c_char {
  checked((|| filesystem(api)?.font_path(text(resources)?))(), error)
    .map_or(core::ptr::null_mut(), |path| owned(&path))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_storage_monospace_font_path(
  api: *const FsApi,
  resources: *const c_char,
  error: *mut *mut c_char,
) -> *mut c_char {
  checked((|| filesystem(api)?.monospace_font_path(text(resources)?))(), error)
    .map_or(core::ptr::null_mut(), |path| owned(&path))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_storage_load(
  api: *const FsApi,
  path: *const c_char,
  error: *mut *mut c_char,
) -> *mut Buffer {
  let result = (|| filesystem(api)?.load_file(text(path)?))();
  checked(result, error)
    .map_or(core::ptr::null_mut(), |bytes| Box::into_raw(Box::new(Buffer(bytes))))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_storage_write(
  api: *const FsApi,
  path: *const c_char,
  data: *const u8,
  length: usize,
  error: *mut *mut c_char,
) -> bool {
  checked((|| filesystem(api)?.atomic_write(text(path)?, bytes(data, length)?))(), error).is_some()
}
#[no_mangle]
pub unsafe extern "C" fn wodex_storage_preserve(
  api: *const FsApi,
  path: *const c_char,
  data: *const u8,
  length: usize,
  error: *mut *mut c_char,
) -> *mut c_char {
  checked((|| filesystem(api)?.preserve_file(text(path)?, bytes(data, length)?))(), error)
    .map_or(core::ptr::null_mut(), |path| owned(&path))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_transport_new(
  api: *const BackendApi,
  ca_bundle: *const c_char,
  error: *mut *mut c_char,
) -> *mut TransportBridge {
  let result = (|| {
    let (runtime, http, _) = services(api)?;
    let value = Transport::new(runtime, http, text(ca_bundle)?.into())?;
    Ok(TransportBridge { value, queued: VecDeque::new(), current: None, error: CString::default() })
  })();
  checked(result, error).map_or(core::ptr::null_mut(), |value| Box::into_raw(Box::new(value)))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_transport_free(value: *mut TransportBridge) {
  if !value.is_null() {
    drop(Box::from_raw(value));
  }
}
#[no_mangle]
pub unsafe extern "C" fn wodex_transport_start(
  value: *mut TransportBridge,
  url: *const c_char,
  key: *const c_char,
  body: *const c_char,
  error: *mut *mut c_char,
) -> bool {
  checked(
    (|| {
      value.as_mut().ok_or("Missing network backend.")?.value.start(
        text(url)?,
        text(key)?,
        text(body)?,
      )
    })(),
    error,
  )
  .is_some()
}

#[no_mangle]
pub unsafe extern "C" fn wodex_transport_start_request(
  value: *mut TransportBridge,
  request: *const c_char,
  error: *mut *mut c_char,
) -> bool {
  checked(
    (|| {
      let request = text(request)?;
      if request.len() > 4 * 1024 * 1024 + 64 * 1024 {
        return Err("The prepared request exceeds its size limit.".into());
      }
      let request = serde_json::from_str(request)
        .map_err(|_| String::from("Invalid prepared request JSON."))?;
      value.as_mut().ok_or("Missing network backend.")?.value.start_request(&request)
    })(),
    error,
  )
  .is_some()
}
#[no_mangle]
pub unsafe extern "C" fn wodex_transport_cancel(value: *mut TransportBridge) {
  if let Some(value) = value.as_mut() {
    value.value.cancel();
  }
}
#[no_mangle]
pub unsafe extern "C" fn wodex_transport_active(value: *const TransportBridge) -> bool {
  value.as_ref().is_some_and(|value| value.value.active())
}
#[no_mangle]
pub unsafe extern "C" fn wodex_transport_poll(value: *mut TransportBridge) -> NativeEvent {
  let mut event = NativeEvent {
    kind: 0,
    data: core::ptr::null(),
    length: 0,
    http_status: 0,
    error: core::ptr::null(),
  };
  let Some(value) = value.as_mut() else { return event };
  if value.queued.is_empty() {
    value.queued.extend(value.value.poll());
  }
  value.current = value.queued.pop_front();
  match &value.current {
    Some(TransportEvent::Data(data)) => {
      event.kind = 1;
      event.data = data.as_ptr();
      event.length = data.len();
    }
    Some(TransportEvent::Finished { http_status, error }) => {
      value.error = CString::new(error.replace('\0', " ")).unwrap_or_default();
      event.kind = 2;
      event.http_status = *http_status;
      event.error = value.error.as_ptr();
    }
    None => {}
  }
  event
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_new(
  api: *const BackendApi,
  version: *const c_char,
  target: *const c_char,
  test_base: *const c_char,
  ca_bundle: *const c_char,
  error: *mut *mut c_char,
) -> *mut Updater {
  let result = (|| {
    let (runtime, http, fs) = services(api)?;
    Updater::new(
      runtime,
      http,
      fs,
      text(version)?.into(),
      text(target)?.into(),
      text(test_base)?.into(),
      text(ca_bundle)?.into(),
    )
  })();
  checked(result, error).map_or(core::ptr::null_mut(), |value| Box::into_raw(Box::new(value)))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_free(value: *mut Updater) {
  if !value.is_null() {
    drop(Box::from_raw(value));
  }
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_check(
  value: *mut Updater,
  repository: *const c_char,
  error: *mut *mut c_char,
) -> bool {
  checked(
    (|| {
      value
        .as_mut()
        .ok_or("Missing update backend.")?
        .check(text(repository)?, crate::update::UpdateChannel::Version)
    })(),
    error,
  )
  .is_some()
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_check_channel(
  value: *mut Updater,
  repository: *const c_char,
  channel: *const c_char,
  error: *mut *mut c_char,
) -> bool {
  let result = (|| {
    let channel = match text(channel)? {
      "version" => crate::update::UpdateChannel::Version,
      "nightly" => crate::update::UpdateChannel::Nightly,
      _ => return Err(String::from("Unknown update channel.")),
    };
    value.as_mut().ok_or("Missing update backend.")?.check(text(repository)?, channel)
  })();
  checked(result, error).is_some()
}

#[no_mangle]
pub unsafe extern "C" fn wodex_updater_download(
  value: *mut Updater,
  error: *mut *mut c_char,
) -> bool {
  checked(
    value
      .as_mut()
      .ok_or_else(|| String::from("Missing update backend."))
      .and_then(Updater::download),
    error,
  )
  .is_some()
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_cancel(value: *mut Updater) {
  if let Some(value) = value.as_mut() {
    value.cancel();
  }
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_active(value: *const Updater) -> bool {
  value.as_ref().is_some_and(Updater::active)
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_status(value: *const Updater) -> *mut c_char {
  let Some(value) = value.as_ref() else { return core::ptr::null_mut() };
  let status = value.status();
  owned(&serde_json::json!({"phase":status.phase as u32,"message":status.message,"version":status.version,
    "notes":status.notes,"downloaded":status.downloaded,"total":status.total,
    "can_download":status.can_download}).to_string())
}
#[no_mangle]
pub unsafe extern "C" fn wodex_updater_pending(value: *const Updater) -> *mut c_char {
  let Some(value) = value.as_ref() else { return core::ptr::null_mut() };
  serde_json::to_string(&value.pending_install())
    .ok()
    .map_or(core::ptr::null_mut(), |value| owned(&value))
}
#[no_mangle]
pub unsafe extern "C" fn wodex_update_install_json(
  api: *const FsApi,
  pending: *const c_char,
  rename_file: Option<unsafe extern "C" fn(*const c_char, *const c_char) -> i32>,
  error: *mut *mut c_char,
) -> bool {
  let result = (|| {
    let fs = filesystem(api)?;
    let pending = serde_json::from_str(text(pending)?)
      .map_err(|_| String::from("Invalid pending update metadata."))?;
    match rename_file {
      Some(rename) => updater::install_update_with_rename(&fs, &pending, rename),
      None => updater::install_update(&fs, &pending),
    }
  })();
  checked(result, error).is_some()
}
#[no_mangle]
pub unsafe extern "C" fn wodex_pending_free(value: *mut PendingHandle) {
  if !value.is_null() {
    drop(Box::from_raw(value));
  }
}
#[no_mangle]
pub unsafe extern "C" fn wodex_pending_install(
  api: *const FsApi,
  value: *const PendingHandle,
  error: *mut *mut c_char,
) -> bool {
  checked(
    (|| {
      updater::install_update(
        &filesystem(api)?,
        &value.as_ref().ok_or("Missing pending update.")?.value,
      )
    })(),
    error,
  )
  .is_some()
}
