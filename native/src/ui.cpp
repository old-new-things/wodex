#include "ui.hpp"
#include "platform.hpp"
#include <algorithm>
#include <stdexcept>

namespace wodex {

std::string preview(const std::string &value, size_t max_bytes) {
  if (value.size() <= max_bytes) {
    return value;
  }
  size_t end = max_bytes;
  while (end > 0 && (static_cast<unsigned char>(value[end]) & 0xc0) == 0x80) {
    --end;
  }
  return value.substr(0, end) + "…";
}

void erase_last_character(std::string &value) {
  if (value.empty()) {
    return;
  }
  size_t end = value.size() - 1;
  while (end > 0 && (static_cast<unsigned char>(value[end]) & 0xc0) == 0x80) {
    --end;
  }
  value.resize(end);
}

Canvas::Canvas(SDL_Renderer *renderer, const Platform &platform)
    : renderer_(renderer), platform_(platform) {
  font(20);
}

Canvas::~Canvas() {
  for (auto &item : textures_) {
    SDL_DestroyTexture(item.second.value);
  }
  for (auto &item : fonts_) {
    TTF_CloseFont(item.second);
  }
}

TTF_Font *Canvas::font(int size) {
  auto found = fonts_.find(size);
  if (found != fonts_.end()) {
    return found->second;
  }
  TTF_Font *result = nullptr;
  if (platform_.font_data() && platform_.font_size()) {
    auto *rw = SDL_RWFromConstMem(platform_.font_data(), static_cast<int>(platform_.font_size()));
    result = TTF_OpenFontRW(rw, 1, size);
  } else {
    result = TTF_OpenFont(platform_.font_path().c_str(), size);
  }
  if (!result) {
    throw std::runtime_error(std::string("Cannot load font: ") + TTF_GetError());
  }
  fonts_[size] = result;
  return result;
}

Canvas::Texture &Canvas::texture(const std::string &value, int size, Color color, int width) {
  std::string key = std::to_string(size) + ":" + std::to_string(width) + ":" +
                    std::to_string(color.r) + ":" + std::to_string(color.g) + ":" +
                    std::to_string(color.b) + ":" + value;
  auto found = textures_.find(key);
  if (found != textures_.end()) {
    found->second.used = ++texture_clock_;
    return found->second;
  }
  SDL_Color sdl_color{color.r, color.g, color.b, color.a};
  auto *surface = width > 0
                      ? TTF_RenderUTF8_Blended_Wrapped(font(size), value.c_str(), sdl_color, width)
                      : TTF_RenderUTF8_Blended(font(size), value.c_str(), sdl_color);
  if (!surface) {
    throw std::runtime_error(std::string("Cannot render text: ") + TTF_GetError());
  }
  Texture entry{SDL_CreateTextureFromSurface(renderer_, surface), surface->w, surface->h,
                static_cast<size_t>(surface->w) * surface->h * 4, ++texture_clock_};
  SDL_FreeSurface(surface);
  if (!entry.value) {
    throw std::runtime_error(SDL_GetError());
  }
  constexpr size_t max_texture_bytes = 16 * 1024 * 1024;
  while (!textures_.empty() && texture_bytes_ + entry.bytes > max_texture_bytes) {
    auto oldest =
        std::min_element(textures_.begin(), textures_.end(), [](const auto &a, const auto &b) {
          return a.second.used < b.second.used;
        });
    texture_bytes_ -= oldest->second.bytes;
    SDL_DestroyTexture(oldest->second.value);
    textures_.erase(oldest);
  }
  texture_bytes_ += entry.bytes;
  return textures_.emplace(std::move(key), entry).first->second;
}

Canvas::Layout &Canvas::layout(const std::string &value, int size, int width) {
  const std::string key = std::to_string(size) + ":" + std::to_string(width) + ":" + value;
  auto found = layouts_.find(key);
  if (found != layouts_.end())
    return found->second;
  Layout result{{}, TTF_FontLineSkip(font(size)) + 4};
  size_t offset = 0;
  while (offset < value.size()) {
    if (value[offset] == '\n') {
      result.lines.emplace_back();
      ++offset;
      continue;
    }
    // Measure bounded text without allocating any pixel surfaces. This keeps
    // long histories cheap even when their contents are outside the viewport.
    size_t candidate_end = std::min(value.size(), offset + 512);
    while (candidate_end < value.size() && candidate_end > offset &&
           (static_cast<unsigned char>(value[candidate_end]) & 0xc0) == 0x80)
      --candidate_end;
    const auto newline = value.find('\n', offset);
    if (newline != std::string::npos)
      candidate_end = std::min(candidate_end, newline);
    const auto candidate = value.substr(offset, candidate_end - offset);
    int extent = 0, count = 0;
    if (TTF_MeasureUTF8(font(size), candidate.c_str(), std::max(1, width), &extent, &count) != 0) {
      throw std::runtime_error(TTF_GetError());
    }
    size_t end = 0;
    for (int index = 0; index < std::max(1, count) && end < candidate.size(); ++index) {
      ++end;
      while (end < candidate.size() && (static_cast<unsigned char>(candidate[end]) & 0xc0) == 0x80)
        ++end;
    }
    if (end < candidate.size()) {
      const auto space = candidate.rfind(' ', end);
      if (space != std::string::npos && space > 0)
        end = space;
    }
    result.lines.push_back(candidate.substr(0, end));
    offset += end;
    if (offset < value.size() && (value[offset] == ' ' || value[offset] == '\n'))
      ++offset;
  }
  if (result.lines.empty())
    result.lines.emplace_back();
  if (layout_bytes_ + value.size() * 2 > 16 * 1024 * 1024) {
    layouts_.clear();
    layout_bytes_ = 0;
  }
  layout_bytes_ += value.size() * 2;
  return layouts_.emplace(key, std::move(result)).first->second;
}

void Canvas::rect(SDL_Rect area, Color color, int radius) {
  SDL_SetRenderDrawColor(renderer_, color.r, color.g, color.b, color.a);
  if (!radius) {
    SDL_RenderFillRect(renderer_, &area);
    return;
  }
  radius = std::min({radius, area.w / 2, area.h / 2});
  SDL_Rect middle{area.x + radius, area.y, area.w - radius * 2, area.h};
  SDL_RenderFillRect(renderer_, &middle);
  for (int dy = 0; dy < area.h; ++dy) {
    int inset = 0;
    if (dy < radius) {
      const int distance = radius - dy;
      inset = radius - static_cast<int>(SDL_sqrt(radius * radius - distance * distance));
    } else if (dy >= area.h - radius) {
      const int distance = dy - (area.h - radius - 1);
      inset = radius - static_cast<int>(SDL_sqrt(radius * radius - distance * distance));
    }
    SDL_RenderDrawLine(renderer_, area.x + inset, area.y + dy, area.x + area.w - inset - 1,
                       area.y + dy);
  }
}

void Canvas::outline(SDL_Rect area, Color color) {
  SDL_SetRenderDrawColor(renderer_, color.r, color.g, color.b, color.a);
  SDL_RenderDrawRect(renderer_, &area);
}

int Canvas::text(const std::string &value, int x, int y, int size, Color color, int width) {
  if (value.empty()) {
    return size + 6;
  }
  auto &entry = texture(value, size, color, width);
  SDL_Rect destination{x, y, entry.width, entry.height};
  SDL_RenderCopy(renderer_, entry.value, nullptr, &destination);
  return entry.height;
}

int Canvas::paragraph(const std::string &value, int x, int y, int size, Color color, int width,
                      bool draw) {
  const auto &lines = layout(value, size, width);
  if (draw) {
    for (const auto &line : lines.lines) {
      if (!line.empty() && y + lines.line_height >= clip_.y && y < clip_.y + clip_.h) {
        text(line, x, y, size, color);
      }
      y += lines.line_height;
    }
  }
  return static_cast<int>(lines.lines.size()) * lines.line_height;
}

void Canvas::clip(SDL_Rect area) {
  clip_ = area;
  SDL_RenderSetClipRect(renderer_, &area);
}

void Canvas::unclip() {
  clip_ = {0, 0, 1280, 720};
  SDL_RenderSetClipRect(renderer_, nullptr);
}

int Canvas::text_width(const std::string &value, int size) {
  int width = 0;
  TTF_SizeUTF8(font(size), value.c_str(), &width, nullptr);
  return width;
}

} // namespace wodex
