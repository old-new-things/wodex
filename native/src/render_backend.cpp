#include "wodex_render.h"
#include "platform.hpp"

#include <SDL.h>
#include <SDL_ttf.h>

#include <exception>

namespace {

void *font_open(const void *context, int32_t size, uint32_t style) {
  if (style > WODEX_FONT_MONOSPACE) {
    SDL_SetError("Unsupported font style.");
    return nullptr;
  }
  try {
    const auto &platform = *static_cast<const wodex::Platform *>(context);
    const auto path = style == WODEX_FONT_MONOSPACE
      ? platform.monospace_font_path() : platform.font_path();
    TTF_Font *font = nullptr;
    if (path.empty() && platform.font_data() && platform.font_size()) {
      auto *stream = SDL_RWFromConstMem(platform.font_data(), static_cast<int>(platform.font_size()));
      if (!stream) return nullptr;
      font = TTF_OpenFontRW(stream, 1, size);
    } else {
      font = TTF_OpenFont(path.c_str(), size);
    }
    if (font) {
      const int native_style = ((style & WODEX_FONT_BOLD) ? TTF_STYLE_BOLD : TTF_STYLE_NORMAL)
        | ((style & WODEX_FONT_ITALIC) ? TTF_STYLE_ITALIC : TTF_STYLE_NORMAL);
      TTF_SetFontStyle(font, native_style);
    }
    return font;
  } catch (const std::exception &error) {
    SDL_SetError("Cannot open font: %s", error.what());
    return nullptr;
  } catch (...) {
    SDL_SetError("Cannot open font.");
    return nullptr;
  }
}

void font_close(void *font) { TTF_CloseFont(static_cast<TTF_Font *>(font)); }
int32_t font_line_skip(void *font) { return TTF_FontLineSkip(static_cast<TTF_Font *>(font)); }
int32_t font_ascent(void *font) { return TTF_FontAscent(static_cast<TTF_Font *>(font)); }

int32_t text_measure(void *font, const char *value, int32_t width, int32_t *count) {
  int extent = 0;
  return TTF_MeasureUTF8(static_cast<TTF_Font *>(font), value, width, &extent, count);
}

int32_t text_size(void *font, const char *value, int32_t *width, int32_t *height) {
  return TTF_SizeUTF8(static_cast<TTF_Font *>(font), value, width, height);
}

void *texture_create(void *renderer, void *font, const char *value, WodexColor color,
                     int32_t width, int32_t *output_width, int32_t *output_height) {
  const SDL_Color native_color{color.r, color.g, color.b, color.a};
  SDL_Surface *surface = width > 0
    ? TTF_RenderUTF8_Blended_Wrapped(static_cast<TTF_Font *>(font), value, native_color, width)
    : TTF_RenderUTF8_Blended(static_cast<TTF_Font *>(font), value, native_color);
  if (!surface) return nullptr;
  *output_width = surface->w;
  *output_height = surface->h;
  auto *texture = SDL_CreateTextureFromSurface(static_cast<SDL_Renderer *>(renderer), surface);
  SDL_FreeSurface(surface);
  return texture;
}

void texture_destroy(void *texture) { SDL_DestroyTexture(static_cast<SDL_Texture *>(texture)); }

int32_t set_color(void *renderer, WodexColor color) {
  return SDL_SetRenderDrawColor(static_cast<SDL_Renderer *>(renderer), color.r, color.g, color.b,
                                color.a);
}

int32_t fill_rect(void *renderer, WodexRect area, WodexColor color) {
  const SDL_Rect rect{area.x, area.y, area.w, area.h};
  if (set_color(renderer, color) != 0) return -1;
  return SDL_RenderFillRect(static_cast<SDL_Renderer *>(renderer), &rect);
}

int32_t outline_rect(void *renderer, WodexRect area, WodexColor color) {
  const SDL_Rect rect{area.x, area.y, area.w, area.h};
  if (set_color(renderer, color) != 0) return -1;
  return SDL_RenderDrawRect(static_cast<SDL_Renderer *>(renderer), &rect);
}

int32_t draw_line(void *renderer, int32_t x1, int32_t y1, int32_t x2, int32_t y2,
                  WodexColor color) {
  if (set_color(renderer, color) != 0) return -1;
  return SDL_RenderDrawLine(static_cast<SDL_Renderer *>(renderer), x1, y1, x2, y2);
}

int32_t copy_texture(void *renderer, void *texture, WodexRect area) {
  const SDL_Rect rect{area.x, area.y, area.w, area.h};
  return SDL_RenderCopy(static_cast<SDL_Renderer *>(renderer), static_cast<SDL_Texture *>(texture),
                         nullptr, &rect);
}

int32_t set_clip(void *renderer, const WodexRect *area) {
  if (!area) return SDL_RenderSetClipRect(static_cast<SDL_Renderer *>(renderer), nullptr);
  const SDL_Rect rect{area->x, area->y, area->w, area->h};
  return SDL_RenderSetClipRect(static_cast<SDL_Renderer *>(renderer), &rect);
}

const char *render_error() { return SDL_GetError(); }

} // namespace

extern "C" WodexRenderApi wodex_render_api(void *renderer, const void *platform) {
  return {renderer, platform, font_open, font_close, font_line_skip, font_ascent, text_measure, text_size,
          texture_create, texture_destroy, fill_rect, outline_rect, draw_line, copy_texture,
          set_clip, render_error};
}
