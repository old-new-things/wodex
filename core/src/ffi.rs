use crate::state::{WodexCore, MAX_COMMAND_BYTES, MAX_FEED_BYTES, MAX_SAVED_BYTES};
use alloc::{boxed::Box, ffi::CString};
use core::{ffi::c_char, slice, str};

/// Read bounded C strings without first walking an unbounded external buffer.
/// As with all C string APIs, the caller guarantees a readable NUL-terminated
/// allocation. The upper bound limits parser work, not pointer validity checks.
unsafe fn bounded_text<'a>(ptr: *const c_char, max_len: usize) -> Result<&'a str, ()> {
  if ptr.is_null() {
    return Err(());
  }
  let mut len = 0;
  while len <= max_len {
    if *ptr.add(len) == 0 {
      return str::from_utf8(slice::from_raw_parts(ptr.cast(), len)).map_err(|_| ());
    }
    len += 1;
  }
  Err(())
}

fn result_string(value: serde_json::Value) -> *mut c_char {
  serde_json::to_vec(&value)
    .ok()
    .and_then(|bytes| CString::new(bytes).ok())
    .map_or(core::ptr::null_mut(), CString::into_raw)
}

#[no_mangle]
pub unsafe extern "C" fn wodex_new(saved_json: *const c_char) -> *mut WodexCore {
  let saved = if saved_json.is_null() {
    None
  } else {
    match bounded_text(saved_json, MAX_SAVED_BYTES) {
      Ok(saved) => Some(saved),
      // A non-UTF-8 or oversized save is still a recoverable corruption report.
      Err(()) => Some("invalid saved data"),
    }
  };
  Box::into_raw(Box::new(WodexCore::new(saved)))
}

#[no_mangle]
pub unsafe extern "C" fn wodex_free(core: *mut WodexCore) {
  if !core.is_null() {
    drop(Box::from_raw(core));
  }
}

#[no_mangle]
pub unsafe extern "C" fn wodex_call(
  core: *mut WodexCore,
  command_json: *const c_char,
) -> *mut c_char {
  let Some(core) = core.as_mut() else { return core::ptr::null_mut() };
  let Ok(command) = bounded_text(command_json, MAX_COMMAND_BYTES) else {
    return result_string(
      serde_json::json!({"ok":false,"error":"Invalid UTF-8, NULL, or oversized command."}),
    );
  };
  result_string(core.call(command))
}

#[no_mangle]
pub unsafe extern "C" fn wodex_feed(
  core: *mut WodexCore,
  data: *const u8,
  len: usize,
) -> *mut c_char {
  let Some(core) = core.as_mut() else { return core::ptr::null_mut() };
  if len > MAX_FEED_BYTES || (data.is_null() && len != 0) {
    return result_string(
      serde_json::json!({"ok":false,"error":"Invalid stream buffer or chunk size."}),
    );
  }
  let bytes = if len == 0 { &[] } else { slice::from_raw_parts(data, len) };
  result_string(core.feed(bytes))
}

#[no_mangle]
pub unsafe extern "C" fn wodex_string_free(value: *mut c_char) {
  if !value.is_null() {
    drop(CString::from_raw(value));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use core::ffi::CStr;

  #[test]
  fn ffi_owns_returned_strings_and_tolerates_nulls() {
    unsafe {
      let core = wodex_new(core::ptr::null());
      let command = CString::new("{\"op\":\"snapshot\"}").unwrap();
      let response = wodex_call(core, command.as_ptr());
      assert!(!response.is_null());
      let value: serde_json::Value =
        serde_json::from_slice(CStr::from_ptr(response).to_bytes()).unwrap();
      assert_eq!(value["ok"], true);
      wodex_string_free(response);
      let invalid = wodex_call(core, core::ptr::null());
      assert!(!invalid.is_null());
      wodex_string_free(invalid);
      wodex_free(core);
      wodex_free(core::ptr::null_mut());
      wodex_string_free(core::ptr::null_mut());
      assert!(wodex_call(core::ptr::null_mut(), command.as_ptr()).is_null());
    }
  }
}
