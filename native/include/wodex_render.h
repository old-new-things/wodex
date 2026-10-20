#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct WodexRect {
  int32_t x, y, w, h;
} WodexRect;

typedef struct WodexColor {
  uint8_t r, g, b, a;
} WodexColor;

// Platform and renderer must remain alive until the Rust canvas is destroyed.
// Callbacks are synchronous and must not retain pointers to text or rectangles.
typedef struct WodexRenderApi {
  void *renderer;
  const void *platform;
  void *(*font_open)(const void *, int32_t);
  void (*font_close)(void *);
  int32_t (*font_line_skip)(void *);
  int32_t (*text_measure)(void *, const char *, int32_t, int32_t *);
  int32_t (*text_size)(void *, const char *, int32_t *, int32_t *);
  void *(*texture_create)(void *, void *, const char *, WodexColor, int32_t,
                         int32_t *, int32_t *);
  void (*texture_destroy)(void *);
  int32_t (*fill_rect)(void *, WodexRect, WodexColor);
  int32_t (*outline_rect)(void *, WodexRect, WodexColor);
  int32_t (*draw_line)(void *, int32_t, int32_t, int32_t, int32_t, WodexColor);
  int32_t (*copy_texture)(void *, void *, WodexRect);
  int32_t (*set_clip)(void *, const WodexRect *);
  const char *(*error)(void);
} WodexRenderApi;

WodexRenderApi wodex_render_api(void *renderer, const void *platform);

// Opaque helpers exercise the production Rust renderer from native font tests.
typedef struct WodexCanvas WodexCanvas;
WodexCanvas *wodex_canvas_new(const WodexRenderApi *api);
void wodex_canvas_free(WodexCanvas *canvas);
int32_t wodex_canvas_paragraph(WodexCanvas *canvas, const char *value, int32_t x, int32_t y,
                               int32_t size, WodexColor color, int32_t width, bool draw);
int32_t wodex_canvas_text_width(WodexCanvas *canvas, const char *value, int32_t size);
void wodex_canvas_clip(WodexCanvas *canvas, WodexRect area);
void wodex_canvas_unclip(WodexCanvas *canvas);
bool wodex_canvas_ok(const WodexCanvas *canvas);

#ifdef __cplusplus
}
#endif
