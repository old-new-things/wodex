use crate::state::{WodexCore, MAX_COMMAND_BYTES, MAX_FEED_BYTES, MAX_SAVED_BYTES};
use crate::update::{self, WodexSha256, MAX_NRO_BYTES, MAX_UPDATE_BYTES, MAX_VERSION_BYTES};
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

#[no_mangle]
pub unsafe extern "C" fn wodex_update_inspect(
  feed_json: *const c_char,
  current_version: *const c_char,
  repository: *const c_char,
) -> *mut c_char {
  let (Ok(feed), Ok(current), Ok(repository)) = (
    bounded_text(feed_json, MAX_UPDATE_BYTES),
    bounded_text(current_version, MAX_VERSION_BYTES),
    bounded_text(repository, 140),
  ) else {
    return result_string(serde_json::json!({
      "ok":false,"available":false,"error":"Invalid, missing, or oversized update metadata."
    }));
  };
  result_string(update::inspect(feed, current, repository))
}

#[no_mangle]
pub extern "C" fn wodex_sha256_new() -> *mut WodexSha256 {
  Box::into_raw(Box::new(WodexSha256::new()))
}

#[no_mangle]
pub unsafe extern "C" fn wodex_sha256_update(
  hash: *mut WodexSha256,
  data: *const u8,
  len: usize,
) -> bool {
  let Some(hash) = hash.as_mut() else { return false };
  if len > MAX_FEED_BYTES || (data.is_null() && len != 0) {
    return false;
  }
  let bytes = if len == 0 { &[] } else { slice::from_raw_parts(data, len) };
  hash.update(bytes)
}

#[no_mangle]
pub unsafe extern "C" fn wodex_sha256_finish(hash: *mut WodexSha256) -> *mut c_char {
  let Some(hash) = hash.as_mut() else { return core::ptr::null_mut() };
  CString::new(hash.finish()).map_or(core::ptr::null_mut(), CString::into_raw)
}

#[no_mangle]
pub unsafe extern "C" fn wodex_sha256_free(hash: *mut WodexSha256) {
  if !hash.is_null() {
    drop(Box::from_raw(hash));
  }
}

#[no_mangle]
pub unsafe extern "C" fn wodex_nro_validate(
  header: *const u8,
  header_len: usize,
  file_size: u64,
) -> bool {
  if header.is_null() || header_len < 0x80 || header_len as u64 > MAX_NRO_BYTES {
    return false;
  }
  update::validate_nro(slice::from_raw_parts(header, header_len), file_size)
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

  #[test]
  fn update_and_hash_ffi_preserve_ownership_and_reject_invalid_inputs() {
    unsafe {
      let metadata = CString::new(
        serde_json::json!({
          "schema_version":1,"channel":"version","build":{
            "version":"0.2.0","size":1000,"notes":"Version 0.2.0",
            "url":"https://example.github.io/wodex/builds/version/0.2.0/wodex.nro",
            "sha256":"ab".repeat(32)
          }
        })
        .to_string(),
      )
      .unwrap();
      let repository = CString::new("example/wodex").unwrap();
      let current = CString::new("0.1.0").unwrap();
      let response = wodex_update_inspect(metadata.as_ptr(), current.as_ptr(), repository.as_ptr());
      let parsed: serde_json::Value =
        serde_json::from_slice(CStr::from_ptr(response).to_bytes()).unwrap();
      assert_eq!(parsed["available"], true);
      assert_eq!(parsed["build"]["version"], "0.2.0");
      wodex_string_free(response);
      let invalid = wodex_update_inspect(core::ptr::null(), current.as_ptr(), repository.as_ptr());
      let parsed: serde_json::Value =
        serde_json::from_slice(CStr::from_ptr(invalid).to_bytes()).unwrap();
      assert_eq!(parsed["ok"], false);
      wodex_string_free(invalid);
      let invalid = wodex_update_inspect(metadata.as_ptr(), current.as_ptr(), core::ptr::null());
      let parsed: serde_json::Value =
        serde_json::from_slice(CStr::from_ptr(invalid).to_bytes()).unwrap();
      assert_eq!(parsed["ok"], false);
      wodex_string_free(invalid);
      let bad_utf8 = [0xff_u8, 0];
      let invalid =
        wodex_update_inspect(bad_utf8.as_ptr().cast(), current.as_ptr(), repository.as_ptr());
      let parsed: serde_json::Value =
        serde_json::from_slice(CStr::from_ptr(invalid).to_bytes()).unwrap();
      assert_eq!(parsed["ok"], false);
      wodex_string_free(invalid);

      let hash = wodex_sha256_new();
      assert!(!hash.is_null());
      assert!(wodex_sha256_update(hash, core::ptr::null(), 0));
      assert!(!wodex_sha256_update(hash, core::ptr::null(), 1));
      assert!(!wodex_sha256_update(hash, b"x".as_ptr(), MAX_FEED_BYTES + 1));
      for byte in b"abc" {
        assert!(wodex_sha256_update(hash, byte, 1));
      }
      for _ in 0..2 {
        let digest = wodex_sha256_finish(hash);
        assert_eq!(
          CStr::from_ptr(digest).to_str().unwrap(),
          "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        wodex_string_free(digest);
      }
      assert!(!wodex_sha256_update(hash, b"x".as_ptr(), 1));
      wodex_sha256_free(hash);
      wodex_sha256_free(core::ptr::null_mut());
      assert!(wodex_sha256_finish(core::ptr::null_mut()).is_null());
      assert!(!wodex_sha256_update(core::ptr::null_mut(), b"abc".as_ptr(), 3));

      let mut header = [0_u8; 0x80];
      header[0x10..0x14].copy_from_slice(b"NRO0");
      for offset in [0x18, 0x24, 0x28, 0x30] {
        header[offset..offset + 4].copy_from_slice(&0x80_u32.to_le_bytes());
      }
      assert!(wodex_nro_validate(header.as_ptr(), header.len(), 0x80));
      assert!(!wodex_nro_validate(header.as_ptr(), 127, 0x80));
      assert!(!wodex_nro_validate(core::ptr::null(), 0, 0x80));
    }
  }
}
