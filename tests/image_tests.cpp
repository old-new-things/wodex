#include "platform.hpp"
#include "storage.hpp"
#include "wodex_app.h"
#include "wodex_core.h"
#include "wodex_host.h"
#include "wodex_render.h"

#include <SDL.h>
#include <SDL_ttf.h>
#include <nlohmann/json.hpp>

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
using Json = nlohmann::json;
using Clock = std::chrono::steady_clock;
std::string loopback;
std::atomic<int> started{0};
std::atomic<int> completed{0};
std::atomic<int> unexpected{0};
int passed = 0;

void check(bool condition, const char *description) {
  if (!condition) throw std::runtime_error(description);
  ++passed;
}

void check_app(bool success, char *error) {
  std::unique_ptr<char, decltype(&wodex_string_free)> owned(error, wodex_string_free);
  if (!success) throw std::runtime_error(error ? error : "Rust application returned no error detail.");
}

// Only this integration binary substitutes the transport address. Production URL
// parsing, image policy, requests, decoding and rendering run without overrides.
bool local_string(void *handle, uint32_t option, const char *value) noexcept {
  const auto *native = wodex_http_api();
  try {
    if (option == 1) {
      const std::string url(value);
      const std::string prefix = "https://image.test/";
      if (url.compare(0, prefix.size(), prefix) != 0) {
        ++unexpected;
        return false;
      }
      const std::string target = loopback + "/" + url.substr(prefix.size());
      return native->set_string(handle, 4, "*") &&
        native->set_string(handle, option, target.c_str());
    }
    return native->set_string(handle, option, option == 5 ? "http,https" : value);
  } catch (...) {
    return false;
  }
}

uint32_t local_perform(void *handle) noexcept {
  ++started;
  const auto result = wodex_http_api()->perform(handle);
  ++completed;
  return result;
}

void seed(const std::string &path, const std::string &content, const char *role = "assistant") {
  auto messages = Json::array();
  if (std::string(role) == "assistant") {
    messages.push_back({
      {"role", "user"}, {"content", "Show the image."}, {"reasoning_content", ""},
      {"status", "complete"}
    });
  }
  messages.push_back({
    {"role", role}, {"content", content}, {"reasoning_content", ""}, {"status", "complete"}
  });
  Json state = {
    {"version", 1},
    {"settings", {
      {"api_key", "stored-key-not-for-images"}, {"base_url", "https://api.deepseek.com"},
      {"model", "deepseek-flash"}, {"thinking", false}, {"max_tokens", 4096},
      {"system_prompt", ""}, {"update_repository", ""}, {"update_channel", "version"}
    }},
    {"conversations", Json::array({{
      {"id", "chat-1"}, {"title", "Images · 图片"},
      {"messages", messages}
    }})},
    {"active_id", "chat-1"}, {"next_id", 2}
  };
  wodex::atomic_write(path, state.dump());
}

class SdlRuntime {
public:
  SdlRuntime() {
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMECONTROLLER) != 0)
      throw std::runtime_error(SDL_GetError());
    if (TTF_Init() != 0) {
      const std::string error = TTF_GetError();
      SDL_Quit();
      throw std::runtime_error(error);
    }
  }
  ~SdlRuntime() { TTF_Quit(); SDL_Quit(); }
};

class AppSession {
public:
  AppSession(wodex::Platform &platform, SDL_Renderer *renderer)
    : bindings_(platform), host_(bindings_.api()),
      render_(wodex_render_api(renderer, &platform)), app_(nullptr, wodex_app_free) {
    backend_.http.set_string = local_string;
    backend_.http.perform = local_perform;
    char *error = nullptr;
    app_.reset(wodex_app_new(&host_, &render_, &backend_, "0.2.1", "", false, false, &error));
    check_app(app_ != nullptr, error);
  }

  void frame() {
    char *error = nullptr;
    const bool ticked = wodex_app_tick(app_.get(), &error);
    check_app(ticked, error);
    error = nullptr;
    const bool rendered = wodex_app_render(app_.get(), &error);
    check_app(rendered, error);
  }

  void run_for(int milliseconds) {
    const auto deadline = Clock::now() + std::chrono::milliseconds(milliseconds);
    do {
      frame();
      SDL_Delay(4);
    } while (Clock::now() < deadline);
  }

private:
  wodex::NativeBindings bindings_;
  WodexBackendApi backend_{wodex_native_fs_api(), *wodex_runtime_api(), *wodex_http_api()};
  WodexHostApi host_;
  WodexRenderApi render_;
  std::unique_ptr<WodexApp, decltype(&wodex_app_free)> app_;
};

struct ImagePixels {
  std::array<int, 4> colors{};
  int left = 1280;
  int top = 720;
  int right = 0;
  int bottom = 0;
  Uint32 center = 0;
  bool visible() const {
    for (const auto count : colors) if (count < 500) return false;
    return true;
  }
};

ImagePixels image_pixels(SDL_Renderer *renderer) {
  std::vector<Uint32> pixels(1280 * 720);
  if (SDL_RenderReadPixels(renderer, nullptr, SDL_PIXELFORMAT_ARGB8888, pixels.data(),
      1280 * static_cast<int>(sizeof(Uint32))) != 0)
    throw std::runtime_error(SDL_GetError());
  ImagePixels result;
  for (int y = 100; y < 590; ++y) {
    for (int x = 310; x < 1250; ++x) {
      const auto pixel = pixels[y * 1280 + x];
      const auto red = (pixel >> 16) & 255;
      const auto green = (pixel >> 8) & 255;
      const auto blue = pixel & 255;
      const int color = red > 200 && green < 80 && blue < 80 ? 0 :
        red < 80 && green > 180 && blue < 100 ? 1 :
        red < 80 && green < 100 && blue > 200 ? 2 :
        red > 200 && green > 170 && blue < 80 ? 3 : -1;
      if (color < 0) continue;
      ++result.colors[color];
      result.left = std::min(result.left, x);
      result.right = std::max(result.right, x);
      result.top = std::min(result.top, y);
      result.bottom = std::max(result.bottom, y);
    }
  }
  if (result.visible()) {
    result.center = pixels[((result.top + result.bottom) / 2) * 1280 +
      (result.left + result.right) / 2];
  }
  return result;
}

void screenshot(SDL_Renderer *renderer, const std::string &path) {
  if (path.empty()) return;
  std::unique_ptr<SDL_Surface, decltype(&SDL_FreeSurface)> surface(
    SDL_CreateRGBSurfaceWithFormat(0, 1280, 720, 24, SDL_PIXELFORMAT_RGB24), SDL_FreeSurface);
  if (!surface || SDL_RenderReadPixels(renderer, nullptr, surface->format->format,
      surface->pixels, surface->pitch) != 0 || SDL_SaveBMP(surface.get(), path.c_str()) != 0)
    throw std::runtime_error(SDL_GetError());
}

void check_image(wodex::Platform &platform, SDL_Renderer *renderer, const std::string &path,
                 const std::string &endpoint, const std::string &output) {
  const auto content = "![Four colors · 四种颜色](https://image.test/" +
    endpoint + ")\n\nThe original Markdown remains in history.";
  seed(path, content);
  const auto initial = started.load();
  AppSession app(platform, renderer);
  const auto deadline = Clock::now() + std::chrono::seconds(10);
  ImagePixels pixels;
  while (Clock::now() < deadline) {
    app.frame();
    pixels = image_pixels(renderer);
    if (pixels.visible()) break;
    SDL_Delay(4);
  }
  screenshot(renderer, output.empty() ? "" : output + "/" + endpoint + ".bmp");
  check(pixels.visible(), "Image did not produce the expected PNG/JPEG color pixels.");
  const auto ratio = static_cast<double>(pixels.right - pixels.left + 1) /
    static_cast<double>(pixels.bottom - pixels.top + 1);
  check(std::abs(ratio - 2.0) < 0.08, "Image display distorted the source aspect ratio.");
  if (endpoint == "color.png")
    check((pixels.center & 0x00ffffff) != 0, "Transparent PNG pixels rendered as opaque black.");
  app.run_for(120);
  check(started.load() == initial + 1, "Repeated rendering downloaded the same image again.");
  const auto state = Json::parse(wodex::load_file(path));
  check(state.at("conversations").at(0).at("messages").back().at("content") == content,
    "Displaying an image rewrote its saved Markdown source.");
}

void check_failure(wodex::Platform &platform, SDL_Renderer *renderer, const std::string &path,
                   const std::string &endpoint) {
  seed(path, "![Unavailable image · 图片不可用](https://image.test/" + endpoint + ")");
  const auto expected = completed.load() + 1;
  const auto initial = started.load();
  AppSession app(platform, renderer);
  const auto deadline = Clock::now() + std::chrono::seconds(10);
  while (completed.load() < expected && Clock::now() < deadline) {
    app.frame();
    SDL_Delay(4);
  }
  check(completed.load() == expected, "Failed image request did not finish promptly.");
  app.run_for(150);
  check(started.load() == initial + 1, "An image failure retried every frame.");
  check(!image_pixels(renderer).visible(), "Invalid image data produced image pixels.");
}

void check_no_request(wodex::Platform &platform, SDL_Renderer *renderer, const std::string &path,
                      const std::string &content, const char *role = "assistant") {
  seed(path, content, role);
  const auto initial = started.load();
  AppSession app(platform, renderer);
  app.run_for(100);
  check(started.load() == initial, "Hidden, invalid or unfinished image source was fetched.");
}
} // namespace

int main(int argc, char **argv) {
  if (argc != 3 && argc != 4) {
    std::cerr << "Usage: wodex_image_tests BASE_URL RESOURCE_DIR [SCREENSHOT_DIR]\n";
    return 1;
  }
  try {
    loopback = argv[1];
    check(loopback.compare(0, 17, "http://127.0.0.1:") == 0, "Expected an isolated loopback server.");
    wodex::Platform platform(argv[2]);
    const auto path = platform.data_dir() + "/state.json";
    const std::string output = argc == 4 ? argv[3] : "";
    SdlRuntime runtime;
    std::unique_ptr<SDL_Window, decltype(&SDL_DestroyWindow)> window(
      SDL_CreateWindow("Wodex image integration", 0, 0, 1280, 720, SDL_WINDOW_HIDDEN), SDL_DestroyWindow);
    if (!window) throw std::runtime_error(SDL_GetError());
    std::unique_ptr<SDL_Renderer, decltype(&SDL_DestroyRenderer)> renderer(
      SDL_CreateRenderer(window.get(), -1, SDL_RENDERER_SOFTWARE), SDL_DestroyRenderer);
    if (!renderer) throw std::runtime_error(SDL_GetError());
    if (SDL_RenderSetLogicalSize(renderer.get(), 1280, 720) != 0)
      throw std::runtime_error(SDL_GetError());
    check_image(platform, renderer.get(), path, "color.png", output);
    check_image(platform, renderer.get(), path, "color.jpg", output);
    {
      seed(path, "![First](https://image.test/duplicate.png)\n\n"
        "![Second](https://image.test/duplicate.png)");
      const auto initial = started.load();
      AppSession app(platform, renderer.get());
      app.run_for(300);
      check(started.load() == initial + 1, "Duplicate Markdown images issued duplicate requests.");
      check(image_pixels(renderer.get()).visible(), "Duplicate image cards did not render.");
    }
    for (const auto *endpoint : {
      "denied.png", "redirect.png", "invalid.png", "truncated.png", "oversized.png", "too-large.png"
    }) check_failure(platform, renderer.get(), path, endpoint);
    for (const auto *source : {
      "![Incomplete](https://image.test/never-requested.png",
      "![Local](file:///secret.png)",
      "![HTTP](http://image.test/never-requested.png)",
      "![Credentials](https://name:secret@image.test/never-requested.png)",
      "![Inline data](data:image/png;base64,AAAA)"
    }) check_no_request(platform, renderer.get(), path, source);
    check_no_request(platform, renderer.get(), path,
      "![Literal user prompt](https://image.test/never-requested.png)", "user");
    std::string offscreen = "![Above the visible area](https://image.test/never-offscreen.png)\n\n";
    for (int line = 0; line < 100; ++line) offscreen += "Text after the hidden image.\n";
    check_no_request(platform, renderer.get(), path, offscreen);
    {
      seed(path, "![Cancel on exit](https://image.test/slow.png)");
      const auto initial = started.load();
      auto app = std::make_unique<AppSession>(platform, renderer.get());
      const auto deadline = Clock::now() + std::chrono::seconds(2);
      while (started.load() == initial && Clock::now() < deadline) {
        app->frame();
        SDL_Delay(4);
      }
      check(started.load() == initial + 1, "Slow image request did not start.");
      app->run_for(80);
      const auto before = Clock::now();
      app.reset();
      check(Clock::now() - before < std::chrono::seconds(2),
        "Application shutdown did not cancel its image worker promptly.");
    }
    check(unexpected.load() == 0, "Image requests bypassed production URL validation.");
    check(started.load() == completed.load(), "Application left an image worker running.");
    std::cout << passed << " image download, decode, SDL rendering and lifecycle checks passed.\n";
    return 0;
  } catch (const std::exception &error) {
    std::cerr << "Image integration: " << error.what() << '\n';
    return 1;
  }
}
