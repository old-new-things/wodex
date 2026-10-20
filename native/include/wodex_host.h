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

typedef struct WodexHostApi {
  void *context;
  uint64_t (*now_ms)(void *);
  bool (*running)(void *);
  void (*begin_frame)(void *);
  WodexHostInput (*poll_input)(void *);
  void (*text_input)(void *, bool);
  WodexHostBytes (*clipboard)(void *);
  bool (*edit_system)(void *, WodexHostBytes, WodexHostBytes, bool, WodexHostBytes *);
  WodexHostBytes (*last_error)(void *);
} WodexHostApi;

#ifdef __cplusplus
}

#include <memory>
#include <string>

namespace wodex {
class Platform;

// SDL and libnx bindings borrowed by the Rust application.
// Worker ownership and all backend state live in Rust.
class NativeBindings {
public:
  explicit NativeBindings(Platform &platform);
  ~NativeBindings();
  NativeBindings(const NativeBindings &) = delete;
  NativeBindings &operator=(const NativeBindings &) = delete;
  WodexHostApi api();

private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};
} // namespace wodex
#endif
