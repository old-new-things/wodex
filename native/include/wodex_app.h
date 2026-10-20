#pragma once

#include "wodex_host.h"
#include "wodex_backend.h"
#include "wodex_render.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct WodexApp WodexApp;

// Tables and their borrowed native contexts must outlive the application.
// The caller invokes these functions on the main thread only. Error strings
// are owned by Rust and released with wodex_string_free from wodex_core.h.
WodexApp *wodex_app_new(const WodexHostApi *host, const WodexRenderApi *render,
                       const WodexBackendApi *backend, const char *version, const char *target, bool demo, bool show_updates, char **error);
bool wodex_app_tick(WodexApp *app, char **error);
bool wodex_app_render(WodexApp *app, char **error);
bool wodex_app_running(const WodexApp *app);
// Offline screenshot/demo callers can suppress image fetches before rendering.
void wodex_app_set_image_loading(WodexApp *app, bool enabled);
void wodex_app_free(WodexApp *app);
WodexPendingUpdate *wodex_app_take_pending(WodexApp *app);

#ifdef __cplusplus
}
#endif
