#include "platform.hpp"
#include "ui.hpp"

#include <SDL.h>
#include <SDL_ttf.h>

#include <array>
#include <cstdlib>
#include <iostream>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

int passed = 0;

void check(bool condition, const char* description) {
  if (!condition) throw std::runtime_error(description);
  ++passed;
}

class FontEnvironment {
public:
  FontEnvironment() {
    const char* value = std::getenv("WODEX_FONT");
    if (value) previous_ = value;
    set("");
  }

  ~FontEnvironment() {
    if (previous_) SDL_setenv("WODEX_FONT", previous_->c_str(), 1);
    else ::unsetenv("WODEX_FONT");
  }

  void set(const std::string& value) {
    if (SDL_setenv("WODEX_FONT", value.c_str(), 1) != 0) {
      throw std::runtime_error("Cannot set the font test environment.");
    }
  }

private:
  std::optional<std::string> previous_;
};

class SdlRuntime {
public:
  SdlRuntime() {
    if (SDL_Init(SDL_INIT_VIDEO) != 0) throw std::runtime_error(SDL_GetError());
    if (TTF_Init() != 0) {
      const std::string error = TTF_GetError();
      SDL_Quit();
      throw std::runtime_error(error);
    }
  }

  ~SdlRuntime() {
    TTF_Quit();
    SDL_Quit();
  }
};

void check_coverage(const std::string& font_path) {
  std::unique_ptr<TTF_Font, decltype(&TTF_CloseFont)> font{
    TTF_OpenFont(font_path.c_str(), 21), &TTF_CloseFont};
  if (!font) throw std::runtime_error(TTF_GetError());
  const std::u32string characters = U"AZaz019中文汉字简体繁體漢語臺灣龘𠮷日本語あいうアイウ한국어，。！？「」…";
  for (const auto character : characters) {
    if (TTF_GlyphIsProvided32(font.get(), static_cast<Uint32>(character)) == 0) {
      throw std::runtime_error("The bundled font is missing codepoint " +
        std::to_string(static_cast<Uint32>(character)) + ".");
    }
    ++passed;
  }
}

void check_rendering(const wodex::Platform& platform) {
  constexpr int surface_width = 640;
  constexpr int surface_height = 320;
  std::unique_ptr<SDL_Surface, decltype(&SDL_FreeSurface)> surface{
    SDL_CreateRGBSurfaceWithFormat(0, surface_width, surface_height, 32, SDL_PIXELFORMAT_RGBA32),
    &SDL_FreeSurface};
  if (!surface) throw std::runtime_error(SDL_GetError());
  std::unique_ptr<SDL_Renderer, decltype(&SDL_DestroyRenderer)> renderer{
    SDL_CreateSoftwareRenderer(surface.get()), &SDL_DestroyRenderer};
  if (!renderer) throw std::runtime_error(SDL_GetError());
  wodex::Canvas canvas{renderer.get(), platform};
  const std::string mixed = "简体中文／繁體漢字，Hello Switch! 日本語あいう 한국어。𠮷\n";
  std::string long_text;
  for (int index = 0; index < 40; ++index) long_text += mixed;
  const SDL_Rect clip{16, 16, 260, 200};

  // These are the UI's reasoning, default, message, and editor point sizes.
  for (const int size : std::array<int, 4>{17, 20, 21, 22}) {
    SDL_SetRenderDrawColor(renderer.get(), 0, 0, 0, 255);
    if (SDL_RenderClear(renderer.get()) != 0) throw std::runtime_error(SDL_GetError());
    const int single_height = canvas.paragraph("中文", 0, 0, size, wodex::ink, clip.w, false);
    check(single_height > size, "CJK line spacing does not contain a full glyph.");
    const int measured = canvas.paragraph(long_text, 0, 0, size, wodex::ink, clip.w, false);
    check(measured > clip.h, "Long mixed-language text did not wrap beyond the viewport.");
    check(measured % single_height == 0, "Wrapped text has inconsistent line spacing.");
    canvas.clip(clip);
    const int rendered = canvas.paragraph(long_text, clip.x, clip.y, size, wodex::ink, clip.w);
    canvas.unclip();
    check(rendered == measured, "Measuring and rendering disagree on CJK paragraph height.");
    check(canvas.paragraph(long_text, 0, 0, size, wodex::ink, clip.w, false) == measured,
      "Cached CJK paragraph metrics changed.");
    check(canvas.text_width("中文", size) > canvas.text_width("中", size),
      "CJK text width does not account for each character.");

    std::vector<Uint32> pixels(surface_width * surface_height);
    if (SDL_RenderReadPixels(renderer.get(), nullptr, SDL_PIXELFORMAT_ARGB8888, pixels.data(),
        surface_width * static_cast<int>(sizeof(Uint32))) != 0) {
      throw std::runtime_error(SDL_GetError());
    }
    bool visible = false;
    for (int y = 0; y < surface_height; ++y) {
      for (int x = 0; x < surface_width; ++x) {
        const bool drawn = (pixels[static_cast<std::size_t>(y * surface_width + x)] & 0x00ffffff) != 0;
        const bool inside = x >= clip.x && x < clip.x + clip.w &&
          y >= clip.y && y < clip.y + clip.h;
        if (drawn && !inside) throw std::runtime_error("Text escaped the chat viewport clip.");
        visible = visible || drawn;
      }
    }
    check(visible, "Mixed-language text produced no visible pixels.");
    ++passed;
  }
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::cerr << "Usage: wodex_font_tests RESOURCE_DIR\n";
    return 1;
  }
  try {
    SdlRuntime runtime;
    FontEnvironment environment;
    const std::string resource_dir = argv[1];
    const std::string bundled_font = resource_dir + "/fonts/NotoSansCJKsc-Regular.otf";
    wodex::Platform platform{resource_dir};
    check(platform.font_path() == bundled_font, "The desktop did not select its bundled CJK font.");
    check(platform.font_data() == nullptr && platform.font_size() == 0,
      "The desktop unnecessarily copied the bundled font into memory.");
    check_coverage(platform.font_path());
    check_rendering(platform);

    environment.set(bundled_font);
    wodex::Platform overridden{resource_dir + "/missing"};
    check(overridden.font_path() == bundled_font, "WODEX_FONT did not override resource discovery.");
    environment.set(bundled_font + ".missing");
    bool rejected = false;
    try {
      wodex::Platform invalid{resource_dir};
    } catch (const std::runtime_error& error) {
      rejected = std::string{error.what()}.find("WODEX_FONT") != std::string::npos;
    }
    check(rejected, "An invalid explicit font override silently selected another font.");
    std::cout << passed << " font and CJK rendering checks passed.\n";
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "Font tests: " << error.what() << '\n';
    return 1;
  }
}
