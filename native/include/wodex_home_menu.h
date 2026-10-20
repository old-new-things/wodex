#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Each fallible callback returns an unmodified libnx Result (zero is success).
// A session is confined to one Rust worker. The adapter owns only service/AES
// handles; package validation, state transitions and rollback belong to Rust.
typedef struct WodexHomeMenuKey {
  uint64_t id;
  uint32_t version;
  uint8_t type;
  uint8_t install_type;
  uint8_t reserved[2];
} WodexHomeMenuKey;

typedef struct WodexHomeMenuApi {
  uint32_t (*open)(void **);
  void (*close)(void *);
  uint32_t (*application_records)(void *, uint32_t, uint64_t *, uint32_t, uint32_t *);
  uint32_t (*metadata_exists)(void *, uint8_t, uint64_t, bool *);
  uint32_t (*content_exists)(void *, const uint8_t *, bool *);
  uint32_t (*free_space)(void *, uint64_t *);
  uint32_t (*seal_header)(void *, uint8_t *, size_t);
  uint32_t (*set_installing)(void *, bool);
  uint32_t (*placeholder_id)(void *, uint8_t *);
  uint32_t (*placeholder_exists)(void *, const uint8_t *, bool *);
  uint32_t (*placeholder_create)(void *, const uint8_t *, const uint8_t *, uint64_t);
  uint32_t (*placeholder_write)(void *, const uint8_t *, uint64_t, const uint8_t *, size_t);
  uint32_t (*placeholder_flush)(void *);
  uint32_t (*placeholder_register)(void *, const uint8_t *, const uint8_t *);
  uint32_t (*placeholder_delete)(void *, const uint8_t *);
  uint32_t (*content_delete)(void *, const uint8_t *);
  uint32_t (*metadata_set)(void *, const WodexHomeMenuKey *, const uint8_t *, size_t);
  uint32_t (*metadata_remove)(void *, const WodexHomeMenuKey *);
  uint32_t (*metadata_commit)(void *);
  uint32_t (*application_register)(void *, const WodexHomeMenuKey *);
} WodexHomeMenuApi;

// Desktop returns NULL and never attempts system title installation.
const WodexHomeMenuApi *wodex_home_menu_api(void);
void wodex_home_menu_configure_exit(void);

#ifdef __cplusplus
}
#endif
