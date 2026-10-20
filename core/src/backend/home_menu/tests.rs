use super::*;
use alloc::{boxed::Box, collections::BTreeSet, vec::Vec};

const TEMPLATE: &[u8] = include_bytes!("../../../../romfs/home-menu/template.bin");

#[derive(Default)]
struct Mock {
  calls: Vec<&'static str>,
  fail_at: usize,
  fail_cleanup: bool,
  ambiguous_registration: bool,
  ambiguous_metadata: bool,
  ambiguous_create: bool,
  ambiguous_content_register: bool,
  fail_readback: bool,
  fail_metadata_readback: bool,
  content: BTreeSet<[u8; 16]>,
  placeholders: BTreeSet<[u8; 16]>,
  next_placeholder: u8,
  metadata: bool,
  application: bool,
  locked: bool,
}

impl Mock {
  fn step(&mut self, name: &'static str) -> u32 {
    self.calls.push(name);
    if self.calls.len() == self.fail_at
      || (self.fail_cleanup && name == "metadata_remove")
      || (self.fail_readback && self.application && name == "list")
      || (self.fail_metadata_readback && self.metadata && name == "metadata_exists")
    {
      0x1234
    } else {
      0
    }
  }
}

unsafe fn mock<'a>(context: *mut c_void) -> &'a mut Mock {
  &mut *context.cast()
}
unsafe fn id(value: *const u8) -> [u8; 16] {
  core::slice::from_raw_parts(value, 16).try_into().unwrap()
}
unsafe extern "C" fn open(out: *mut *mut c_void) -> u32 {
  *out = Box::into_raw(Box::new(Mock::default())).cast();
  0
}
unsafe extern "C" fn close(context: *mut c_void) {
  drop(Box::<Mock>::from_raw(context.cast()));
}
unsafe extern "C" fn list(
  context: *mut c_void,
  _: u32,
  out: *mut u64,
  _: u32,
  count: *mut u32,
) -> u32 {
  let state = mock(context);
  let rc = state.step("list");
  *count = u32::from(state.application);
  if state.application {
    *out = TITLE_ID;
  }
  rc
}
unsafe extern "C" fn metadata_exists(context: *mut c_void, _: u8, _: u64, out: *mut bool) -> u32 {
  let state = mock(context);
  let rc = state.step("metadata_exists");
  *out = state.metadata;
  rc
}
unsafe extern "C" fn content_exists(context: *mut c_void, value: *const u8, out: *mut bool) -> u32 {
  let state = mock(context);
  let rc = state.step("content_exists");
  *out = state.content.contains(&id(value));
  rc
}
unsafe extern "C" fn space(context: *mut c_void, out: *mut u64) -> u32 {
  *out = 64 * 1024 * 1024;
  mock(context).step("space")
}
unsafe extern "C" fn seal(context: *mut c_void, bytes: *mut u8, len: usize) -> u32 {
  assert_eq!(len, 0xc00);
  let rc = mock(context).step("seal");
  if rc == 0 {
    // A deliberately non-cryptographic test transform makes dependent hashes
    // change while keeping tests independent of keys or Switch hardware.
    for byte in core::slice::from_raw_parts_mut(bytes, len) {
      *byte ^= 0xa5;
    }
  }
  rc
}
unsafe extern "C" fn lock(context: *mut c_void, enabled: bool) -> u32 {
  let state = mock(context);
  let rc = state.step(if enabled { "lock" } else { "unlock" });
  if rc == 0 {
    state.locked = enabled;
  }
  rc
}
unsafe extern "C" fn generate(context: *mut c_void, out: *mut u8) -> u32 {
  let state = mock(context);
  let rc = state.step("generate");
  state.next_placeholder += 1;
  core::slice::from_raw_parts_mut(out, 16).fill(state.next_placeholder);
  rc
}
unsafe extern "C" fn create(
  context: *mut c_void,
  _: *const u8,
  placeholder: *const u8,
  _: u64,
) -> u32 {
  let state = mock(context);
  let rc = state.step("create");
  if rc == 0 || state.ambiguous_create {
    assert!(state.placeholders.insert(id(placeholder)));
  }
  rc
}
unsafe extern "C" fn has_placeholder(
  context: *mut c_void,
  value: *const u8,
  out: *mut bool,
) -> u32 {
  let state = mock(context);
  let rc = state.step("placeholder_exists");
  *out = state.placeholders.contains(&id(value));
  rc
}
unsafe extern "C" fn write(
  context: *mut c_void,
  placeholder: *const u8,
  _: u64,
  _: *const u8,
  size: usize,
) -> u32 {
  let state = mock(context);
  assert!(state.placeholders.contains(&id(placeholder)));
  assert!(size <= COPY_CHUNK);
  state.step("write")
}
unsafe extern "C" fn flush(context: *mut c_void) -> u32 {
  mock(context).step("flush")
}
unsafe extern "C" fn register(
  context: *mut c_void,
  content: *const u8,
  placeholder: *const u8,
) -> u32 {
  let state = mock(context);
  let rc = state.step("register");
  if rc == 0 || state.ambiguous_content_register {
    assert!(state.placeholders.remove(&id(placeholder)));
    assert!(state.content.insert(id(content)));
  }
  rc
}
unsafe extern "C" fn delete_placeholder(context: *mut c_void, value: *const u8) -> u32 {
  let state = mock(context);
  let rc = state.step("placeholder_delete");
  if rc == 0 {
    state.placeholders.remove(&id(value));
  }
  rc
}
unsafe extern "C" fn delete_content(context: *mut c_void, value: *const u8) -> u32 {
  let state = mock(context);
  assert!(!state.metadata && !state.application, "never delete referenced content");
  let rc = state.step("content_delete");
  if rc == 0 {
    state.content.remove(&id(value));
  }
  rc
}
unsafe extern "C" fn set_metadata(
  context: *mut c_void,
  key: *const MetaKey,
  _: *const u8,
  size: usize,
) -> u32 {
  let state = mock(context);
  assert_eq!((*key).id, TITLE_ID);
  assert_eq!(size, 96);
  assert_eq!(state.content.len(), 3, "all content precedes metadata");
  let rc = state.step("metadata_set");
  if rc == 0 || state.ambiguous_metadata {
    state.metadata = true;
  }
  rc
}
unsafe extern "C" fn remove_metadata(context: *mut c_void, _: *const MetaKey) -> u32 {
  let state = mock(context);
  let rc = state.step("metadata_remove");
  if rc == 0 {
    state.metadata = false;
  }
  rc
}
unsafe extern "C" fn commit(context: *mut c_void) -> u32 {
  mock(context).step("commit")
}
unsafe extern "C" fn application(context: *mut c_void, _: *const MetaKey) -> u32 {
  let state = mock(context);
  assert!(state.metadata && state.content.len() == 3);
  let rc = state.step("application_register");
  if rc == 0 || state.ambiguous_registration {
    state.application = true;
  }
  rc
}
fn api() -> HomeMenuApi {
  HomeMenuApi {
    open,
    close,
    application_records: list,
    metadata_exists,
    content_exists,
    free_space: space,
    seal_header: seal,
    set_installing: lock,
    placeholder_id: generate,
    placeholder_exists: has_placeholder,
    placeholder_create: create,
    placeholder_write: write,
    placeholder_flush: flush,
    placeholder_register: register,
    placeholder_delete: delete_placeholder,
    content_delete: delete_content,
    metadata_set: set_metadata,
    metadata_remove: remove_metadata,
    metadata_commit: commit,
    application_register: application,
  }
}
fn session(state: Mock) -> Session {
  Session { api: api(), handle: Box::into_raw(Box::new(state)).cast() }
}
fn prepared() -> Package {
  let mut package = Package::parse(TEMPLATE.to_vec()).unwrap();
  let session = session(Mock::default());
  package.seal(|data| session.seal(data)).unwrap();
  package
}

#[test]
fn bundled_template_is_key_free_bounded_and_materializes_all_dependent_hashes() {
  verify_bundle(TEMPLATE).unwrap();
  let mut package = Package::parse(TEMPLATE.to_vec()).unwrap();
  let session = session(Mock::default());
  package.seal(|data| session.seal(data)).unwrap();
  assert!(package.size() < 1024 * 1024);
  for (content, id) in package.contents().zip(package.ids()) {
    use sha2::{Digest, Sha256};
    assert_eq!(&Sha256::digest(content)[..16], id);
    assert_ne!(&content[0x200..0x204], b"NCA3");
  }
  let database = package.database();
  assert_eq!(&database[..4], &[16, 0, 3, 0]);
  for (slot, index) in [2, 0, 1].into_iter().enumerate() {
    assert_eq!(&database[24 + slot * 24..40 + slot * 24], &package.ids()[index]);
  }
  assert!(package.seal(|_| Ok(())).is_err());
}

#[test]
fn compiled_manifest_rejects_changed_program_payload_and_unexpected_bundle_size() {
  let mut changed = TEMPLATE.to_vec();
  changed[20 + 0xd00] ^= 1;
  assert!(verify_bundle(&changed).is_err());
  assert!(verify_bundle(&TEMPLATE[..TEMPLATE.len() - 1]).is_err());
  changed = TEMPLATE.to_vec();
  changed.extend_from_slice(b"untrusted");
  assert!(verify_bundle(&changed).is_err());
}

#[test]
fn rejects_truncated_overlapping_wrong_title_encrypted_and_corrupt_templates() {
  for end in [0, 8, 19, 20, 300, TEMPLATE.len() - 1] {
    assert!(Package::parse(TEMPLATE[..end].to_vec()).is_err());
  }
  for offset in [0, 8, 20 + 0x210, 20 + 0x300, 20 + 0x404, 20 + 0x280, TEMPLATE.len() - 412] {
    let mut bytes = TEMPLATE.to_vec();
    bytes[offset] ^= 0xff;
    assert!(Package::parse(bytes).is_err(), "corruption at {offset}");
  }
  let mut extra = TEMPLATE.to_vec();
  extra.push(0);
  assert!(Package::parse(extra).is_err());
}

#[test]
fn metadata_write_error_after_side_effect_is_reconciled_before_content_cleanup() {
  let package = prepared();
  let baseline = session(Mock::default());
  install_package(&baseline, &package).unwrap();
  let fail_at =
    unsafe { mock(baseline.handle) }.calls.iter().position(|call| *call == "metadata_set").unwrap()
      + 1;
  let session = session(Mock { fail_at, ambiguous_metadata: true, ..Mock::default() });
  assert!(install_package(&session, &package).is_err());
  let state = unsafe { mock(session.handle) };
  assert!(!state.metadata && state.content.is_empty());
  let removed = state.calls.iter().position(|call| *call == "metadata_remove").unwrap();
  let deleted = state.calls.iter().position(|call| *call == "content_delete").unwrap();
  assert!(removed < deleted);
}

#[test]
fn uncertain_metadata_after_failed_write_retains_content() {
  let package = prepared();
  let baseline = session(Mock::default());
  install_package(&baseline, &package).unwrap();
  let fail_at =
    unsafe { mock(baseline.handle) }.calls.iter().position(|call| *call == "metadata_set").unwrap()
      + 1;
  let session = session(Mock {
    fail_at,
    ambiguous_metadata: true,
    fail_metadata_readback: true,
    ..Mock::default()
  });
  assert!(install_package(&session, &package).unwrap_err().contains("Cleanup could not finish"));
  let state = unsafe { mock(session.handle) };
  assert!(state.metadata && state.content.len() == 3);
  assert!(!state.calls.contains(&"content_delete"));
}

#[test]
fn ambiguous_placeholder_creation_and_registration_are_reconciled_without_leaks() {
  let package = prepared();
  let baseline = session(Mock::default());
  install_package(&baseline, &package).unwrap();
  for operation in ["create", "register"] {
    let fail_at =
      unsafe { mock(baseline.handle) }.calls.iter().position(|call| *call == operation).unwrap()
        + 1;
    let session = session(Mock {
      fail_at,
      ambiguous_create: operation == "create",
      ambiguous_content_register: operation == "register",
      ..Mock::default()
    });
    assert!(install_package(&session, &package).is_err());
    let state = unsafe { mock(session.handle) };
    assert!(state.placeholders.is_empty() && state.content.is_empty());
  }
}

#[test]
fn installs_content_then_metadata_then_home_record_and_verifies_readback() {
  let session = session(Mock::default());
  install_package(&session, &prepared()).unwrap();
  let state = unsafe { mock(session.handle) };
  assert_eq!(state.content.len(), 3);
  assert!(state.metadata && state.application && !state.locked);
  assert!(state.placeholders.is_empty());
  assert!(
    state.calls.iter().position(|c| *c == "metadata_set").unwrap()
      < state.calls.iter().position(|c| *c == "application_register").unwrap()
  );
}

#[test]
fn every_pre_registration_failure_rolls_back_only_new_content() {
  let package = prepared();
  let baseline = session(Mock::default());
  install_package(&baseline, &package).unwrap();
  let end = unsafe { mock(baseline.handle) }
    .calls
    .iter()
    .position(|call| *call == "application_register")
    .unwrap()
    + 1;
  for fail_at in 1..=end {
    let session = session(Mock { fail_at, ..Mock::default() });
    assert!(install_package(&session, &package).is_err(), "failure {fail_at}");
    let state = unsafe { mock(session.handle) };
    assert!(state.content.is_empty(), "content after failure {fail_at}: {:?}", state.calls);
    assert!(state.placeholders.is_empty(), "placeholder after failure {fail_at}");
    assert!(!state.metadata && !state.application && !state.locked);
  }
}

#[test]
fn refuses_content_collision_without_deleting_existing_objects() {
  let package = prepared();
  let mut state = Mock::default();
  state.content.insert(package.ids()[1]);
  let session = session(state);
  assert!(install_package(&session, &package).unwrap_err().contains("occupied"));
  let state = unsafe { mock(session.handle) };
  assert_eq!(state.content.iter().copied().collect::<Vec<_>>(), [package.ids()[1]]);
  assert!(state.placeholders.is_empty());
}

#[test]
fn failed_metadata_cleanup_retains_referenced_content_and_reports_recovery() {
  let package = prepared();
  let baseline = session(Mock::default());
  install_package(&baseline, &package).unwrap();
  let fail_at = unsafe { mock(baseline.handle) }
    .calls
    .iter()
    .position(|call| *call == "application_register")
    .unwrap()
    + 1;
  let session = session(Mock { fail_at, fail_cleanup: true, ..Mock::default() });
  assert!(install_package(&session, &package).unwrap_err().contains("Cleanup could not finish"));
  let state = unsafe { mock(session.handle) };
  assert_eq!(state.content.len(), 3);
  assert!(state.metadata);
}

#[test]
fn ambiguous_ns_registration_keeps_everything_even_when_readback_fails() {
  let package = prepared();
  let baseline = session(Mock::default());
  install_package(&baseline, &package).unwrap();
  let fail_at = unsafe { mock(baseline.handle) }
    .calls
    .iter()
    .position(|call| *call == "application_register")
    .unwrap()
    + 1;
  for fail_readback in [false, true] {
    let session =
      session(Mock { fail_at, ambiguous_registration: true, fail_readback, ..Mock::default() });
    assert!(install_package(&session, &package).unwrap_err().contains("content was retained"));
    let state = unsafe { mock(session.handle) };
    assert!(state.application && state.metadata);
    assert_eq!(state.content.len(), 3);
    assert!(!state.calls.contains(&"metadata_remove"));
  }
}
