#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// The adapter translates ABI-specific POSIX flags, stat layouts and errno into
// these values. All storage policy and retry decisions live in Rust.
enum WodexFsOpenMode {
  WODEX_FS_READ = 0,
  WODEX_FS_CREATE_NEW = 1,
  WODEX_FS_DIRECTORY = 2,
  WODEX_FS_READ_FOLLOW = 3
};

enum WodexFsKind {
  WODEX_FS_REGULAR = 1,
  WODEX_FS_DIRECTORY_KIND = 2,
  WODEX_FS_SYMLINK = 3,
  WODEX_FS_OTHER = 4
};

enum WodexFsErrorKind {
  WODEX_FS_MISSING = 1,
  WODEX_FS_EXISTS = 2,
  WODEX_FS_INTERRUPTED = 3,
  WODEX_FS_ERROR = 4
};

typedef struct WodexFileInfo {
  uint32_t kind;
  uint64_t size;
} WodexFileInfo;

typedef struct WodexFsApi {
  bool is_switch;
  bool is_apple;
  int32_t (*open)(const char *, uint32_t);
  int64_t (*read)(int32_t, uint8_t *, size_t);
  int64_t (*write)(int32_t, const uint8_t *, size_t);
  int32_t (*seek)(int32_t, uint64_t);
  int32_t (*close)(int32_t);
  int32_t (*sync)(int32_t);
  int32_t (*fstat)(int32_t, WodexFileInfo *);
  int32_t (*stat)(const char *, bool, WodexFileInfo *);
  int32_t (*rename)(const char *, const char *);
  int32_t (*unlink)(const char *);
  int32_t (*mkdir)(const char *);
  int32_t (*chmod)(const char *, uint32_t);
  int32_t (*fchmod)(int32_t, uint32_t);
  uint32_t (*error_kind)(void);
  const char *(*error_message)(void);
  const char *(*getenv)(const char *);
  uint64_t (*pid)(void);
} WodexFsApi;

WodexFsApi wodex_native_fs_api(void);

#ifdef __cplusplus
}
#endif
