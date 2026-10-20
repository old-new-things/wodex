#include "platform.hpp"
#include "storage.hpp"
#include "wodex_app.h"
#include "wodex_core.h"
#include "wodex_host.h"
#include "wodex_render.h"

#include <SDL.h>
#include <SDL_ttf.h>
#include <nlohmann/json.hpp>

#include <chrono>
#include <cstring>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
using Json = nlohmann::json;
using Clock = std::chrono::steady_clock;
constexpr const char *stored_key = "disk-local-test-key";
int passed = 0;

void check(bool condition, const char *description) {
  if (!condition) throw std::runtime_error(description);
  ++passed;
}

void check_app(bool success, char *error) {
  std::unique_ptr<char, decltype(&wodex_string_free)> owned(error, wodex_string_free);
  if (!success) throw std::runtime_error(error ? error : "Rust application returned no error detail.");
}

Json core_call(WodexCore *core, const Json &command) {
  std::unique_ptr<char, decltype(&wodex_string_free)> response(
    wodex_call(core, command.dump().c_str()), wodex_string_free);
  if (!response) throw std::runtime_error("Rust core returned no response.");
  auto result = Json::parse(response.get());
  check(result.value("ok", false), "Could not seed the app's settings.");
  return result;
}

void seed(const std::string &path, const std::string &base_url) {
  std::unique_ptr<WodexCore, decltype(&wodex_free)> core(wodex_new(nullptr), wodex_free);
  check(core != nullptr, "Cannot create the seed conversation.");
  core_call(core.get(), {{"op", "settings"}, {"settings", {
    {"api_key", stored_key}, {"base_url", base_url + "/normal"},
    {"model", "mock-model"}, {"thinking", true}
  }}});
  const auto exported = core_call(core.get(), {{"op", "export"}});
  wodex::atomic_write(path, exported.at("data").dump());
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
  AppSession(wodex::Platform &platform, SDL_Renderer *renderer, bool updates = false)
    : bindings_(platform), host_(bindings_.api()),
      render_(wodex_render_api(renderer, &platform)), app_(nullptr, wodex_app_free) {
    char *error = nullptr;
    app_.reset(wodex_app_new(&host_, &render_, &backend_, "0.2.0", "", false, updates, &error));
    check_app(app_ != nullptr, error);
    frame();
  }

  void frame() {
    char *error = nullptr;
    const bool ticked = wodex_app_tick(app_.get(), &error);
    check_app(ticked, error);
    error = nullptr;
    const bool rendered = wodex_app_render(app_.get(), &error);
    check_app(rendered, error);
  }

  void key(SDL_Keycode value, Uint16 modifiers = KMOD_NONE) {
    SDL_Event event{};
    event.type = SDL_KEYDOWN;
    event.key.keysym.sym = value;
    event.key.keysym.mod = modifiers;
    push(event);
  }

  void text(const char *value) {
    SDL_Event event{};
    event.type = SDL_TEXTINPUT;
    if (std::strlen(value) >= sizeof(event.text.text))
      throw std::runtime_error("Text fixture exceeds an SDL text event.");
    std::strcpy(event.text.text, value);
    push(event);
  }

  void click(int x, int y) {
    SDL_Event event{};
    event.type = SDL_MOUSEBUTTONDOWN;
    event.button.button = SDL_BUTTON_LEFT;
    event.button.x = x;
    event.button.y = y;
    push(event);
  }

  void close() {
    SDL_Event event{};
    event.type = SDL_QUIT;
    push(event);
    check(!wodex_app_running(app_.get()), "SDL quit did not close the Rust application.");
  }

private:
  void push(SDL_Event &event) {
    if (SDL_PushEvent(&event) != 1) throw std::runtime_error(SDL_GetError());
    frame();
  }
  wodex::NativeBindings bindings_;
  WodexBackendApi backend_{wodex_native_fs_api(), *wodex_runtime_api(), *wodex_http_api(), nullptr, nullptr};
  WodexHostApi host_;
  WodexRenderApi render_;
  std::unique_ptr<WodexApp, decltype(&wodex_app_free)> app_;
};

Json saved_state(const std::string &path) {
  return Json::parse(wodex::load_file(path));
}

Json wait_for_response(AppSession &app, const std::string &path) {
  const auto deadline = Clock::now() + std::chrono::seconds(10);
  while (Clock::now() < deadline) {
    app.frame();
    auto state = saved_state(path);
    const auto &messages = state.at("conversations").at(0).at("messages");
    if (messages.size() == 2 && messages.back().value("status", "") == "complete")
      return state;
    SDL_Delay(4);
  }
  throw std::runtime_error("The Rust application did not persist a completed streamed response.");
}

void check_response(const Json &state) {
  const auto &messages = state.at("conversations").at(0).at("messages");
  check(messages.size() == 2, "Retry duplicated the user's message.");
  check(messages.at(0).at("content") == "Hello, Switch!",
    "Cancelling an editor changed the committed draft.");
  check(messages.at(1).at("content") == "你好，Switch! 🌿",
    "The Rust application lost fragmented Chinese response text.");
  check(messages.at(1).at("reasoning_content") == "思考中…",
    "The Rust application lost the streamed reasoning text.");
  check(messages.at(1).at("usage").at("total_tokens") == 15,
    "The application did not persist streamed token usage.");
  check(state.at("settings").at("api_key") == stored_key,
    "The application wrote the environment API key into persistent settings.");
  check(state.at("settings").at("update_repository") == "old-new-things/wodex",
    "The application lost the official GitHub Pages update source.");
  check(state.at("settings").at("update_channel") == "version",
    "The application enabled experimental nightly updates without an explicit selection.");
}

void check_chat_pixels(SDL_Renderer *renderer) {
  constexpr SDL_Rect viewport{344, 94, 850, 360};
  std::vector<Uint32> pixels(viewport.w * viewport.h);
  if (SDL_RenderReadPixels(renderer, &viewport, SDL_PIXELFORMAT_ARGB8888, pixels.data(),
      viewport.w * static_cast<int>(sizeof(Uint32))) != 0)
    throw std::runtime_error(SDL_GetError());
  std::size_t bright = 0;
  for (auto pixel : pixels) {
    const auto red = (pixel >> 16) & 255;
    const auto green = (pixel >> 8) & 255;
    const auto blue = pixel & 255;
    if (red > 150 && green > 150 && blue > 150) ++bright;
  }
  check(bright > 100, "The restored Chinese chat produced no readable text pixels.");
}
} // namespace

int main(int argc, char **argv) {
  if (argc != 3) {
    std::cerr << "Usage: wodex_app_tests BASE_URL RESOURCE_DIR\n";
    return 1;
  }
  try {
    wodex::Platform platform(argv[2]);
    const std::string path = platform.data_dir() + "/state.json";
    seed(path, argv[1]);
    SdlRuntime runtime;
    std::unique_ptr<SDL_Window, decltype(&SDL_DestroyWindow)> window(
      SDL_CreateWindow("Wodex app integration", 0, 0, 1280, 720, SDL_WINDOW_HIDDEN), SDL_DestroyWindow);
    if (!window) throw std::runtime_error(SDL_GetError());
    std::unique_ptr<SDL_Renderer, decltype(&SDL_DestroyRenderer)> renderer(
      SDL_CreateRenderer(window.get(), -1, SDL_RENDERER_SOFTWARE), SDL_DestroyRenderer);
    if (!renderer) throw std::runtime_error(SDL_GetError());
    if (SDL_RenderSetLogicalSize(renderer.get(), 1280, 720) != 0)
      throw std::runtime_error(SDL_GetError());
    {
      AppSession app(platform, renderer.get());
      app.key(SDLK_x);
      app.text("Hello, Switch!");
      app.key(SDLK_RETURN);
      app.key(SDLK_x);
      app.text(" discarded edit");
      app.key(SDLK_ESCAPE);
      app.key(SDLK_RETURN, KMOD_CTRL);
      check_response(wait_for_response(app, path));
      check_chat_pixels(renderer.get());

      app.key(SDLK_F1);
      app.click(450, 550); // Settings: retry last response.
      check_response(wait_for_response(app, path));
      app.key(SDLK_F1);
      app.click(1120, 85); // Settings: updates.
      app.click(580, 252); // Select optional nightly builds.
      check(saved_state(path).at("settings").at("update_channel") == "nightly",
        "Selecting Nightly did not persist the update channel.");
      app.click(400, 252); // Restore published versions.
      check(saved_state(path).at("settings").at("update_channel") == "version",
        "Selecting Versions did not restore the default channel.");
      app.click(1140, 188); // Change the update source.
      app.key(SDLK_a, KMOD_CTRL);
      app.key(SDLK_BACKSPACE);
      app.key(SDLK_RETURN);
      check(saved_state(path).at("settings").at("update_repository") == "",
        "Clearing the update source did not disable checks.");
      app.click(970, 188); // Restore the official source without using the keyboard.
      check(saved_state(path).at("settings").at("update_repository") == "old-new-things/wodex",
        "Use Wodex did not restore the official update source.");
      app.close();
    }
    {
      AppSession restored(platform, renderer.get());
      check_response(saved_state(path));
      check_chat_pixels(renderer.get());
      restored.close();
    }
    std::cout << passed << " Rust app, SDL input, transport, and persistence checks passed.\n";
    return 0;
  } catch (const std::exception &error) {
    std::cerr << "Application integration: " << error.what() << '\n';
    return 1;
  }
}
