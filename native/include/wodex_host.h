#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Borrowed UTF-8/byte buffers remain valid until the next callback on context.
// No callback throws; fallible operations return false and expose last_error.
typedef struct WodexHostBytes {
  const uint8_t *data;
  size_t len;
} WodexHostBytes;

typedef struct WodexHostInput {
  uint32_t kind;
  uint32_t key;
  uint32_t modifiers;
  int32_t x;
  int32_t y;
  int32_t value;
  WodexHostBytes text;
} WodexHostInput;

typedef struct WodexHostTransportEvent {
  uint32_t kind;
  WodexHostBytes data;
  uint16_t http_status;
  WodexHostBytes error;
} WodexHostTransportEvent;

typedef struct WodexHostUpdateStatus {
  uint32_t phase;
  WodexHostBytes message;
  WodexHostBytes version;
  WodexHostBytes notes;
  uint64_t downloaded;
  uint64_t total;
} WodexHostUpdateStatus;

typedef struct WodexHostApi {
  void *context;
  uint64_t (*now_ms)(void *);
  bool (*running)(void *);
  bool (*is_switch)(void *);
  bool (*load)(void *, WodexHostBytes *);
  bool (*save)(void *, WodexHostBytes);
  bool (*preserve)(void *, WodexHostBytes);
  WodexHostBytes (*environment_key)(void *);
  bool (*start)(void *, WodexHostBytes, WodexHostBytes, WodexHostBytes);
  void (*cancel)(void *);
  bool (*network_active)(void *);
  WodexHostTransportEvent (*poll_transport)(void *);
  WodexHostUpdateStatus (*update_status)(void *);
  bool (*update_supported)(void *);
  bool (*update_active)(void *);
  bool (*update_check)(void *, WodexHostBytes);
  bool (*update_download)(void *);
  void (*update_cancel)(void *);
  bool (*prepare_install)(void *, bool *);
  void (*begin_frame)(void *);
  WodexHostInput (*poll_input)(void *);
  void (*text_input)(void *, bool);
  WodexHostBytes (*clipboard)(void *);
  bool (*edit_system)(void *, WodexHostBytes, WodexHostBytes, bool, WodexHostBytes *);
  WodexHostBytes (*last_error)(void *);
} WodexHostApi;

#ifdef __cplusplus
}

#include "updater.hpp"
#include <memory>
#include <optional>
#include <string>

namespace wodex {
class Platform;

// Platform service lifetime is owned by main. Dropping this object joins workers
// before Platform releases libnx sockets and the mounted executable's RomFS.
class NativeBackend {
public:
  NativeBackend(Platform &platform, std::string version, std::string update_target);
  ~NativeBackend();
  NativeBackend(const NativeBackend &) = delete;
  NativeBackend &operator=(const NativeBackend &) = delete;
  WodexHostApi api();
  std::optional<PendingUpdate> pending_install() const;

private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};
} // namespace wodex
#endif
