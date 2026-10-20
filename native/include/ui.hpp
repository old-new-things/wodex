#pragma once

#include <SDL.h>
#include <SDL_ttf.h>
#include <map>
#include <string>
#include <unordered_map>
#include <vector>

namespace wodex {

struct Color {
  Uint8 r, g, b, a = 255;
};

inline constexpr Color background{18, 22, 28};
inline constexpr Color panel{25, 30, 37};
inline constexpr Color elevated{34, 41, 49};
inline constexpr Color ink{237, 241, 238};
inline constexpr Color muted{143, 155, 164};
inline constexpr Color accent{187, 239, 129};
inline constexpr Color dark_ink{27, 40, 25};
inline constexpr Color border{49, 59, 66};
inline constexpr Color danger{255, 167, 158};

class Platform;

class Canvas {
public:
  Canvas(SDL_Renderer *renderer, const Platform &platform);
  ~Canvas();
  void rect(SDL_Rect area, Color color, int radius = 0);
  void outline(SDL_Rect area, Color color);
  int text(const std::string &value, int x, int y, int size, Color color, int width = 0);
  int paragraph(const std::string &value, int x, int y, int size, Color color, int width,
                bool draw = true);
  void clip(SDL_Rect area);
  void unclip();
  int text_width(const std::string &value, int size);

private:
  struct Texture {
    SDL_Texture *value;
    int width;
    int height;
    size_t bytes;
    size_t used;
  };
  struct Layout {
    std::vector<std::string> lines;
    int line_height;
  };
  TTF_Font *font(int size);
  Texture &texture(const std::string &value, int size, Color color, int width);
  Layout &layout(const std::string &value, int size, int width);
  SDL_Renderer *renderer_;
  const Platform &platform_;
  std::map<int, TTF_Font *> fonts_;
  std::unordered_map<std::string, Texture> textures_;
  std::unordered_map<std::string, Layout> layouts_;
  size_t texture_bytes_ = 0;
  size_t texture_clock_ = 0;
  size_t layout_bytes_ = 0;
  SDL_Rect clip_{0, 0, 1280, 720};
};

std::string preview(const std::string &value, size_t max_bytes);
void erase_last_character(std::string &value);

} // namespace wodex
