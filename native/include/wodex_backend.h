#pragma once

#include "wodex_fs.h"
#include "wodex_http.h"
#include "wodex_runtime.h"
#include "wodex_home_menu.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct WodexBackendApi {
  WodexFsApi fs;
  WodexRuntimeApi runtime;
  WodexHttpApi http;
  const WodexHomeMenuApi *home_menu;
} WodexBackendApi;

typedef struct WodexBuffer WodexBuffer;
const uint8_t *wodex_buffer_data(const WodexBuffer *buffer);
size_t wodex_buffer_len(const WodexBuffer *buffer);
void wodex_buffer_free(WodexBuffer *buffer);
char *wodex_storage_data_dir(const WodexFsApi *api, char **error);
char *wodex_storage_font_path(const WodexFsApi *api, const char *resources, char **error);
char *wodex_storage_monospace_font_path(const WodexFsApi *api, const char *resources, char **error);
WodexBuffer *wodex_storage_load(const WodexFsApi *api, const char *path, char **error);
bool wodex_storage_write(const WodexFsApi *api, const char *path,
                         const uint8_t *data, size_t length, char **error);
char *wodex_storage_preserve(const WodexFsApi *api, const char *path,
                            const uint8_t *data, size_t length, char **error);

typedef struct WodexTransport WodexTransport;
typedef struct WodexTransportEvent {
  uint32_t kind;
  const uint8_t *data;
  size_t length;
  uint16_t http_status;
  const char *error;
} WodexTransportEvent;
WodexTransport *wodex_transport_new(const WodexBackendApi *api, const char *ca_bundle, char **error);
void wodex_transport_free(WodexTransport *transport);
bool wodex_transport_start(WodexTransport *transport, const char *url, const char *key,
                           const char *body, char **error);
void wodex_transport_cancel(WodexTransport *transport);
bool wodex_transport_active(const WodexTransport *transport);
WodexTransportEvent wodex_transport_poll(WodexTransport *transport);

typedef struct WodexUpdater WodexUpdater;
WodexUpdater *wodex_updater_new(const WodexBackendApi *api, const char *version,
                               const char *target, const char *test_base,
                               const char *ca_bundle, char **error);
void wodex_updater_free(WodexUpdater *updater);
bool wodex_updater_check(WodexUpdater *updater, const char *repository, char **error);
// channel must be "version" or "nightly". The legacy check selects "version".
bool wodex_updater_check_channel(WodexUpdater *updater, const char *repository, const char *channel, char **error);
bool wodex_updater_download(WodexUpdater *updater, char **error);
void wodex_updater_cancel(WodexUpdater *updater);
bool wodex_updater_active(const WodexUpdater *updater);
char *wodex_updater_status(const WodexUpdater *updater);
char *wodex_updater_pending(const WodexUpdater *updater);
bool wodex_update_install_json(const WodexFsApi *api, const char *pending,
                               int (*rename_file)(const char *, const char *), char **error);

typedef struct WodexPendingUpdate WodexPendingUpdate;
void wodex_pending_free(WodexPendingUpdate *pending);
bool wodex_pending_install(const WodexFsApi *api, const WodexPendingUpdate *pending, char **error);

#ifdef __cplusplus
}
#endif
