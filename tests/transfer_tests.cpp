// Real Rust app + native sockets, controlled by the isolated PC test client.
#include "platform.hpp"
#include "storage.hpp"
#include "wodex_app.h"
#include "wodex_core.h"
#include "wodex_host.h"
#include "wodex_render.h"
#include <SDL.h>
#include <SDL_ttf.h>
#include <nlohmann/json.hpp>
#include <cstring>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>

namespace {
using Json = nlohmann::json;
std::string pairing_code;
uint16_t listener_port = 0;

bool loopback(uint8_t *address) {
  const uint8_t value[4] = {127, 0, 0, 1};
  std::memcpy(address, value, sizeof(value));
  return true;
}
bool capture_random(uint8_t *bytes, size_t size) {
  if (!wodex_transfer_api()->random(bytes, size)) return false;
  static constexpr char hex[] = "0123456789abcdef";
  pairing_code.clear();
  for (size_t index = 0; index < size; ++index) {
    pairing_code += hex[bytes[index] >> 4];
    pairing_code += hex[bytes[index] & 15];
  }
  return true;
}
int32_t capture_listen(const uint8_t *address, uint16_t *port) {
  const int32_t fd = wodex_transfer_api()->listen(address, port);
  if (fd >= 0) listener_port = *port;
  return fd;
}
void checked(bool success, char *error) {
  std::unique_ptr<char, decltype(&wodex_string_free)> owned(error, wodex_string_free);
  if (!success) throw std::runtime_error(error ? error : "Application operation failed.");
}
Json call(WodexCore *core, const Json &command) {
  std::unique_ptr<char, decltype(&wodex_string_free)> result(
    wodex_call(core, command.dump().c_str()), wodex_string_free);
  if (!result) throw std::runtime_error("Cannot seed transfer fixture.");
  Json value = Json::parse(result.get());
  if (!value.value("ok", false)) throw std::runtime_error("Invalid seed command.");
  return value;
}
void seed(const std::string &path) {
  std::unique_ptr<WodexCore, decltype(&wodex_free)> core(wodex_new(nullptr), wodex_free);
  call(core.get(), {{"op", "settings"}, {"settings", {
    {"api_key", "saved-transfer-test-key"}, {"system_prompt", "你好，PC transfer"}
  }}});
  wodex::atomic_write(path, call(core.get(), {{"op", "export"}}).at("data").dump());
}
class SdlRuntime {
public:
  SdlRuntime() {
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMECONTROLLER) != 0)
      throw std::runtime_error(SDL_GetError());
    if (TTF_Init() != 0) {
      SDL_Quit();
      throw std::runtime_error(TTF_GetError());
    }
  }
  ~SdlRuntime() { TTF_Quit(); SDL_Quit(); }
};
}

int main(int argc, char **argv) {
  if (argc != 2) return 1;
  try {
    wodex::Platform platform(argv[1]);
    seed(platform.data_dir() + "/state.json");
    SdlRuntime sdl;
    std::unique_ptr<SDL_Window, decltype(&SDL_DestroyWindow)> window(
      SDL_CreateWindow("Wodex transfer test", 0, 0, 1280, 720, SDL_WINDOW_HIDDEN),
      SDL_DestroyWindow);
    std::unique_ptr<SDL_Renderer, decltype(&SDL_DestroyRenderer)> renderer(
      window ? SDL_CreateRenderer(window.get(), -1, SDL_RENDERER_SOFTWARE) : nullptr,
      SDL_DestroyRenderer);
    if (!renderer) throw std::runtime_error(SDL_GetError());
    if (SDL_RenderSetLogicalSize(renderer.get(), 1280, 720) != 0)
      throw std::runtime_error(SDL_GetError());
    wodex::NativeBindings bindings(platform);
    auto host = bindings.api();
    auto render = wodex_render_api(renderer.get(), &platform);
    auto transfer = *wodex_transfer_api();
    transfer.local_ipv4 = loopback;
    transfer.random = capture_random;
    transfer.listen = capture_listen;
    WodexBackendApi backend{
      wodex_native_fs_api(), *wodex_runtime_api(), *wodex_http_api(), nullptr, &transfer};
    char *error = nullptr;
    std::unique_ptr<WodexApp, decltype(&wodex_app_free)> app(
      wodex_app_new(&host, &render, &backend, "0.2.1", "", false, false, &error), wodex_app_free);
    checked(app != nullptr, error);
    wodex_app_set_image_loading(app.get(), false);
    wodex_app_show_transfer(app.get());
    const auto frame = [&]() {
      char *failure = nullptr;
      const bool ticked = wodex_app_tick(app.get(), &failure);
      checked(ticked, failure);
      failure = nullptr;
      const bool rendered = wodex_app_render(app.get(), &failure);
      checked(rendered, failure);
    };
    const auto click = [&](int x, int y) {
      SDL_Event event{};
      event.type = SDL_MOUSEBUTTONDOWN;
      event.button.button = SDL_BUTTON_LEFT;
      event.button.x = x;
      event.button.y = y;
      if (SDL_PushEvent(&event) != 1) throw std::runtime_error(SDL_GetError());
      frame();
    };
    frame();
    std::cout << "{\"ready\":true}" << std::endl;
    std::string command;
    while (std::getline(std::cin, command)) {
      if (command == "start" || command == "stop") click(440, 555);
      else if (command == "confirm") click(1010, 555);
      else if (command == "reject") click(690, 555);
      else if (command == "frame") frame();
      else if (command == "back") click(1110, 85);
      else if (command == "show") { wodex_app_show_transfer(app.get()); frame(); }
      else if (command.rfind("shot ", 0) == 0) {
        frame();
        std::unique_ptr<SDL_Surface, decltype(&SDL_FreeSurface)> surface(
          SDL_CreateRGBSurfaceWithFormat(0, 1280, 720, 32, SDL_PIXELFORMAT_RGBA32),
          SDL_FreeSurface);
        if (!surface || SDL_RenderReadPixels(renderer.get(), nullptr,
            SDL_PIXELFORMAT_RGBA32, surface->pixels, surface->pitch) != 0 ||
            SDL_SaveBMP(surface.get(), command.substr(5).c_str()) != 0)
          throw std::runtime_error(SDL_GetError());
      }
      else if (command == "quit") {
        SDL_Event event{};
        event.type = SDL_QUIT;
        SDL_PushEvent(&event);
        frame();
        if (wodex_app_running(app.get())) throw std::runtime_error("Transfer blocked app exit.");
        std::cout << "{\"closed\":true}" << std::endl;
        break;
      } else throw std::runtime_error("Unknown integration command.");
      std::cout << Json{{"url", "http://127.0.0.1:" + std::to_string(listener_port)},
        {"code", pairing_code}}.dump() << std::endl;
    }
    return 0;
  } catch (const std::exception &error) {
    std::cerr << "PC transfer integration: " << error.what() << '\n';
    return 1;
  }
}
