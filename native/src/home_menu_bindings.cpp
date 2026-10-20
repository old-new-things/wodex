#include "wodex_home_menu.h"

#ifdef __SWITCH__
#include <switch.h>
#include <cstring>
#include <limits>
#include <new>

extern "C" u32 __nx_applet_exit_mode;

namespace {
constexpr u64 title_id = 0x05776F6465780000ULL;
constexpr Result bad_input = MAKERESULT(Module_Libnx, LibnxError_BadInput);

struct Session {
  bool ncm_ready = false;
  bool ns_ready = false;
  bool crypto_ready = false;
  bool locked = false;
  Service manager{};
  NcmContentStorage storage{};
  NcmContentMetaDatabase sd{};
  NcmContentMetaDatabase user{};
  u8 header_key[32]{};
};

void erase(void *data, size_t size) {
  auto *bytes = static_cast<volatile u8 *>(data);
  while (size--) *bytes++ = 0;
}

void close_session(void *handle) {
  auto *session = static_cast<Session *>(handle);
  if (!session) return;
  if (session->locked) {
    appletSetMediaPlaybackState(false);
    appletUnlockExit();
  }
  ncmContentMetaDatabaseClose(&session->user);
  ncmContentMetaDatabaseClose(&session->sd);
  ncmContentStorageClose(&session->storage);
  serviceClose(&session->manager);
  if (session->crypto_ready) splCryptoExit();
  if (session->ns_ready) nsExit();
  if (session->ncm_ready) ncmExit();
  erase(session->header_key, sizeof(session->header_key));
  delete session;
}

Result open_session(void **out) {
  if (!out) return bad_input;
  *out = nullptr;
  if (hosversionBefore(3, 0, 0))
    return MAKERESULT(Module_Libnx, LibnxError_IncompatSysVer);
  auto *session = new (std::nothrow) Session{};
  if (!session) return MAKERESULT(Module_Libnx, LibnxError_OutOfMemory);
  Result rc = ncmInitialize();
  if (R_SUCCEEDED(rc)) {
    session->ncm_ready = true;
    rc = nsInitialize();
  }
  if (R_SUCCEEDED(rc)) {
    session->ns_ready = true;
    rc = nsGetApplicationManagerInterface(&session->manager);
  }
  if (R_SUCCEEDED(rc))
    rc = ncmOpenContentStorage(&session->storage, NcmStorageId_SdCard);
  if (R_SUCCEEDED(rc))
    rc = ncmOpenContentMetaDatabase(&session->sd, NcmStorageId_SdCard);
  if (R_SUCCEEDED(rc))
    rc = ncmOpenContentMetaDatabase(&session->user, NcmStorageId_BuiltInUser);
  if (R_FAILED(rc)) close_session(session);
  else *out = session;
  return rc;
}

Session *get(void *handle) { return static_cast<Session *>(handle); }
NcmContentId content_id(const u8 *value) {
  NcmContentId id{};
  std::memcpy(id.c, value, sizeof(id.c));
  return id;
}
NcmPlaceHolderId placeholder_id(const u8 *value) {
  NcmPlaceHolderId id{};
  std::memcpy(&id, value, sizeof(id));
  return id;
}
NcmContentMetaKey meta_key(const WodexHomeMenuKey *value) {
  NcmContentMetaKey key{};
  static_assert(sizeof(key) == sizeof(*value));
  static_assert(offsetof(NcmContentMetaKey, id) == offsetof(WodexHomeMenuKey, id));
  static_assert(offsetof(NcmContentMetaKey, version) == offsetof(WodexHomeMenuKey, version));
  static_assert(offsetof(NcmContentMetaKey, type) == offsetof(WodexHomeMenuKey, type));
  static_assert(offsetof(NcmContentMetaKey, install_type) ==
                offsetof(WodexHomeMenuKey, install_type));
  std::memcpy(&key, value, sizeof(key));
  return key;
}

Result application_records(void *, u32 offset, u64 *out, u32 capacity, u32 *count) {
  if (!out || !count || capacity == 0 || capacity > 32 ||
      offset > static_cast<u32>(std::numeric_limits<s32>::max())) return bad_input;
  NsApplicationRecord records[32]{};
  s32 actual = 0;
  const Result rc = nsListApplicationRecord(records, capacity, offset, &actual);
  if (R_SUCCEEDED(rc)) {
    if (actual < 0 || static_cast<u32>(actual) > capacity) return bad_input;
    for (s32 index = 0; index < actual; ++index) out[index] = records[index].application_id;
    *count = actual;
  }
  return rc;
}

Result metadata_exists(void *handle, u8 storage, u64 id, bool *out) {
  if (!out || (storage != NcmStorageId_SdCard && storage != NcmStorageId_BuiltInUser))
    return bad_input;
  auto *db = storage == NcmStorageId_SdCard ? &get(handle)->sd : &get(handle)->user;
  NcmContentMetaKey key{};
  s32 total = 0, count = 0;
  const Result rc = ncmContentMetaDatabaseList(
      db, &total, &count, &key, 1, NcmContentMetaType_Unknown,
      id, id, id, NcmContentInstallType_Full);
  if (R_SUCCEEDED(rc)) *out = total > 0;
  return rc;
}

Result content_exists(void *handle, const u8 *id, bool *out) {
  const auto value = content_id(id);
  return ncmContentStorageHas(&get(handle)->storage, out, &value);
}
Result free_space(void *handle, u64 *out) {
  s64 size = 0;
  const Result rc = ncmContentStorageGetFreeSpaceSize(&get(handle)->storage, &size);
  if (R_SUCCEEDED(rc)) {
    if (size < 0) return bad_input;
    *out = static_cast<u64>(size);
  }
  return rc;
}

// Public key sources are inputs to the console's SPL service, not production
// keys. Derived key material remains in this session and is erased on close.
Result seal_header(void *handle, u8 *data, size_t size) {
  if (!data || size != 0xC00) return bad_input;
  auto *session = get(handle);
  if (!session->crypto_ready) {
    Result rc = splCryptoInitialize();
    if (R_FAILED(rc)) return rc;
    session->crypto_ready = true;
    static constexpr u8 kek_source[16] = {
      0x1f, 0x12, 0x91, 0x3a, 0x4a, 0xcb, 0xf0, 0x0d,
      0x4c, 0xde, 0x3a, 0xf6, 0xd5, 0x23, 0x88, 0x2a
    };
    static constexpr u8 key_source[32] = {
      0x5a, 0x3e, 0xd8, 0x4f, 0xde, 0xc0, 0xd8, 0x26,
      0x31, 0xf7, 0xe2, 0x5d, 0x19, 0x7b, 0xf5, 0xd0,
      0x1c, 0x9b, 0x7b, 0xfa, 0xf6, 0x28, 0x18, 0x3d,
      0x71, 0xf6, 0x4d, 0x73, 0xf1, 0x50, 0xb9, 0xd2
    };
    u8 kek[16]{};
    rc = splCryptoGenerateAesKek(kek_source, 0, 0, kek);
    if (R_SUCCEEDED(rc))
      rc = splCryptoGenerateAesKey(kek, key_source, session->header_key);
    if (R_SUCCEEDED(rc))
      rc = splCryptoGenerateAesKey(kek, key_source + 16, session->header_key + 16);
    erase(kek, sizeof(kek));
    if (R_FAILED(rc)) return rc;
  }
  Aes128XtsContext context{};
  aes128XtsContextCreate(&context, session->header_key, session->header_key + 16, true);
  for (size_t offset = 0; offset < size; offset += 0x200) {
    aes128XtsContextResetSector(&context, offset / 0x200, true);
    aes128XtsEncrypt(&context, data + offset, data + offset, 0x200);
  }
  erase(&context, sizeof(context));
  return 0;
}

Result set_installing(void *handle, bool active) {
  auto *session = get(handle);
  if (active == session->locked) return 0;
  if (active) {
    const Result rc = appletLockExit();
    if (R_FAILED(rc)) return rc;
    session->locked = true;
    appletSetMediaPlaybackState(true);
    return 0;
  }
  appletSetMediaPlaybackState(false);
  const Result rc = appletUnlockExit();
  if (R_SUCCEEDED(rc)) session->locked = false;
  return rc;
}
Result generate_placeholder(void *handle, u8 *out) {
  NcmPlaceHolderId value{};
  const Result rc = ncmContentStorageGeneratePlaceHolderId(&get(handle)->storage, &value);
  if (R_SUCCEEDED(rc)) std::memcpy(out, &value, sizeof(value));
  return rc;
}
Result has_placeholder(void *handle, const u8 *placeholder, bool *out) {
  const auto id = placeholder_id(placeholder);
  return ncmContentStorageHasPlaceHolder(&get(handle)->storage, out, &id);
}
Result create_placeholder(void *handle, const u8 *content, const u8 *placeholder, u64 size) {
  if (size > static_cast<u64>(std::numeric_limits<s64>::max())) return bad_input;
  const auto id = content_id(content);
  const auto temporary = placeholder_id(placeholder);
  return ncmContentStorageCreatePlaceHolder(&get(handle)->storage, &id, &temporary, size);
}
Result write_placeholder(void *handle, const u8 *placeholder, u64 offset,
                         const u8 *data, size_t size) {
  const auto id = placeholder_id(placeholder);
  return ncmContentStorageWritePlaceHolder(&get(handle)->storage, &id, offset, data, size);
}
Result flush_placeholders(void *handle) {
  return ncmContentStorageFlushPlaceHolder(&get(handle)->storage);
}
Result register_placeholder(void *handle, const u8 *content, const u8 *placeholder) {
  const auto id = content_id(content);
  const auto temporary = placeholder_id(placeholder);
  return ncmContentStorageRegister(&get(handle)->storage, &id, &temporary);
}
Result delete_placeholder(void *handle, const u8 *placeholder) {
  const auto id = placeholder_id(placeholder);
  return ncmContentStorageDeletePlaceHolder(&get(handle)->storage, &id);
}
Result delete_content(void *handle, const u8 *content) {
  const auto id = content_id(content);
  return ncmContentStorageDelete(&get(handle)->storage, &id);
}
Result set_metadata(void *handle, const WodexHomeMenuKey *key,
                    const u8 *data, size_t size) {
  const auto native_key = meta_key(key);
  return ncmContentMetaDatabaseSet(&get(handle)->sd, &native_key, data, size);
}
Result remove_metadata(void *handle, const WodexHomeMenuKey *key) {
  const auto native_key = meta_key(key);
  return ncmContentMetaDatabaseRemove(&get(handle)->sd, &native_key);
}
Result commit_metadata(void *handle) {
  return ncmContentMetaDatabaseCommit(&get(handle)->sd);
}
Result register_application(void *handle, const WodexHomeMenuKey *key) {
  // NS IApplicationManagerInterface PushApplicationRecord, command 16.
  // Layout is a u8 event followed by an aligned u64 application ID.
  struct Input { u8 event; u8 padding[7]; u64 id; } input{3, {}, key->id};
  struct Record { NcmContentMetaKey key; u64 storage; } record{
      meta_key(key), NcmStorageId_SdCard};
  static_assert(sizeof(input) == 16 && sizeof(record) == 24);
  SfDispatchParams parameters{};
  parameters.buffer_attrs.attr0 = SfBufferAttr_HipcMapAlias | SfBufferAttr_In;
  parameters.buffers[0] = {&record, sizeof(record)};
  return serviceDispatchImpl(&get(handle)->manager, 16, &input, sizeof(input),
                             nullptr, 0, parameters);
}

const WodexHomeMenuApi api{
  open_session, close_session, application_records, metadata_exists,
  content_exists, free_space, seal_header, set_installing, generate_placeholder, has_placeholder,
  create_placeholder, write_placeholder, flush_placeholders, register_placeholder,
  delete_placeholder, delete_content, set_metadata, remove_metadata,
  commit_metadata, register_application
};
} // namespace

extern "C" const WodexHomeMenuApi *wodex_home_menu_api(void) { return &api; }
extern "C" void wodex_home_menu_configure_exit(void) {
  u64 id = 0;
  if (!envIsNso() &&
      R_SUCCEEDED(svcGetInfo(&id, InfoType_ProgramId, CUR_PROCESS_HANDLE, 0)) &&
      id == title_id) __nx_applet_exit_mode = 1;
}
#else
extern "C" const WodexHomeMenuApi *wodex_home_menu_api(void) { return nullptr; }
extern "C" void wodex_home_menu_configure_exit(void) {}
#endif
