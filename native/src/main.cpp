#include "platform.hpp"
#include "wodex_backend.h"
#include "wodex_app.h"
#include "wodex_core.h"
#include "wodex_host.h"
#include "wodex_home_menu.h"
#include "wodex_render.h"
#include <SDL.h>
#include <SDL_ttf.h>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#ifdef __SWITCH__
#include <switch.h>
#endif

using namespace wodex;

namespace {
// Scope ordering keeps Rust font handles and network workers alive only while
// SDL, libnx services, and the executable's mounted resources remain available.
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
  ~SdlRuntime() {
    TTF_Quit();
    SDL_Quit();
  }
  SdlRuntime(const SdlRuntime &) = delete;
  SdlRuntime &operator=(const SdlRuntime &) = delete;
};

void check_app(bool success, char *error) {
  std::unique_ptr<char, decltype(&wodex_string_free)> owned(error, wodex_string_free);
  if (!success)
    throw std::runtime_error(error ? error : "The application ran out of memory.");
}

void screenshot(SDL_Renderer *renderer, const std::string &path) {
  int width = 0, height = 0;
  if (SDL_GetRendererOutputSize(renderer, &width, &height) != 0 || width <= 0 || height <= 0)
    throw std::runtime_error("Cannot determine screenshot size.");
  std::unique_ptr<SDL_Surface, decltype(&SDL_FreeSurface)> surface(
      SDL_CreateRGBSurfaceWithFormat(0, width, height, 32, SDL_PIXELFORMAT_RGBA32), SDL_FreeSurface);
  if (!surface || SDL_RenderReadPixels(renderer, nullptr, SDL_PIXELFORMAT_RGBA32,
                                      surface->pixels, surface->pitch) != 0)
    throw std::runtime_error("Cannot read screenshot.");
  std::unique_ptr<SDL_Surface, decltype(&SDL_FreeSurface)> output(
      SDL_CreateRGBSurfaceWithFormat(0, 1280, 720, 24, SDL_PIXELFORMAT_RGB24), SDL_FreeSurface);
  if (!output || SDL_BlitScaled(surface.get(), nullptr, output.get(), nullptr) != 0 ||
      SDL_SaveBMP(output.get(), path.c_str()) != 0)
    throw std::runtime_error("Cannot save screenshot.");
}

} // namespace

int main(int argc, char **argv) {
  bool demo = false;
  bool show_updates = false;
  bool show_home_menu = false;
  bool show_transfer = false;
  std::string screenshot_path;
  for (int index = 1; index < argc; ++index) {
    const std::string argument = argv[index];
    if (argument == "--demo")
      demo = true;
    else if (argument == "--updates")
      show_updates = true;
    else if (argument == "--home-menu")
      show_home_menu = true;
    else if (argument == "--transfer")
      show_transfer = true;
    else if (argument == "--screenshot" && index + 1 < argc)
      screenshot_path = argv[++index];
    else if (argument == "--help") {
      std::cout << "Wodex · a native chat client for Nintendo Switch\n"
                   "  --demo                  Sample chat, no network or saves\n"
                   "  --updates               Open update settings\n"
                   "  --home-menu             Open HOME Menu installation settings\n"
                   "  --transfer              Open PC transfer settings\n"
                   "  --screenshot FILE.bmp   Render once and save a 1280×720 image\n"
                   "Environment: DEEPSEEK_API_KEY, OPENAI_API_KEY, WODEX_DATA_DIR, WODEX_FONT, WODEX_CA_BUNDLE\n";
      return 0;
    } else {
      std::cerr << "Unknown argument. Use --help.\n";
      return 1;
    }
  }
  try {
#ifdef __SWITCH__
    // Reject the unsupported memory mode before SDL, networking, or font
    // caches are initialized. The console error path remains available.
    if (appletGetAppletType() == AppletType_LibraryApplet)
      throw std::runtime_error(
        "Wodex requires full application memory.\n"
        "Hold R while launching a game, open hbmenu, then start Wodex.\n"
        "Launching hbmenu from Album is not supported.");
#endif
    std::unique_ptr<WodexPendingUpdate, decltype(&wodex_pending_free)> pending_update(nullptr, wodex_pending_free);
    const auto fs_api = wodex_native_fs_api();
    const WodexBackendApi backend_api{
      fs_api, *wodex_runtime_api(), *wodex_http_api(), wodex_home_menu_api(), wodex_transfer_api()};
    wodex_home_menu_configure_exit();
    const char *update_target = "";
#ifdef __SWITCH__
    if (argc > 0 && argv && argv[0]) update_target = argv[0];
#endif
    {
      std::string resource_dir;
#ifndef __SWITCH__
      std::unique_ptr<char, decltype(&SDL_free)> base_path(SDL_GetBasePath(), SDL_free);
      if (!base_path)
        throw std::runtime_error("Cannot locate the application resource directory.");
      resource_dir = std::string(base_path.get()) + "resources";
#endif
      Platform platform(resource_dir);
      SdlRuntime sdl;
      SDL_SetHint(SDL_HINT_RENDER_SCALE_QUALITY, "1");
      SDL_SetHint(SDL_HINT_TOUCH_MOUSE_EVENTS, "0");
      SDL_SetHint(SDL_HINT_MOUSE_TOUCH_EVENTS, "0");
      std::unique_ptr<SDL_Window, decltype(&SDL_DestroyWindow)> window(
          SDL_CreateWindow("Wodex · Chat for Switch", SDL_WINDOWPOS_CENTERED,
                           SDL_WINDOWPOS_CENTERED, 1280, 720,
                           SDL_WINDOW_RESIZABLE | SDL_WINDOW_ALLOW_HIGHDPI),
          SDL_DestroyWindow);
      if (!window)
        throw std::runtime_error(SDL_GetError());
      auto *renderer_ptr = SDL_CreateRenderer(window.get(), -1,
                                              SDL_RENDERER_ACCELERATED | SDL_RENDERER_PRESENTVSYNC);
      if (!renderer_ptr)
        renderer_ptr = SDL_CreateRenderer(window.get(), -1, SDL_RENDERER_SOFTWARE);
      std::unique_ptr<SDL_Renderer, decltype(&SDL_DestroyRenderer)> renderer(renderer_ptr,
                                                                           SDL_DestroyRenderer);
      if (!renderer)
        throw std::runtime_error(SDL_GetError());
      if (SDL_RenderSetLogicalSize(renderer.get(), 1280, 720) != 0 ||
          SDL_SetRenderDrawBlendMode(renderer.get(), SDL_BLENDMODE_BLEND) != 0)
        throw std::runtime_error(SDL_GetError());
      {
        NativeBindings bindings(platform);
        auto host_api = bindings.api();
        auto render_api = wodex_render_api(renderer.get(), &platform);
        {
          char *error = nullptr;
          std::unique_ptr<WodexApp, decltype(&wodex_app_free)> app(
              wodex_app_new(&host_api, &render_api, &backend_api, WODEX_VERSION, update_target, demo, show_updates, &error),
              wodex_app_free);
          check_app(app != nullptr, error);
          if (show_home_menu)
            wodex_app_show_home_menu(app.get());
          if (show_transfer)
            wodex_app_show_transfer(app.get());
          if (!screenshot_path.empty())
            wodex_app_set_image_loading(app.get(), false);
          do {
            error = nullptr;
            const bool ticked = wodex_app_tick(app.get(), &error);
            check_app(ticked, error);
            error = nullptr;
            const bool rendered = wodex_app_render(app.get(), &error);
            check_app(rendered, error);
            if (!screenshot_path.empty()) {
              screenshot(renderer.get(), screenshot_path);
              break;
            }
            SDL_RenderPresent(renderer.get());
            SDL_Delay(8);
          } while (wodex_app_running(app.get()));
          pending_update.reset(wodex_app_take_pending(app.get()));
        }
      }
    }
    // Every Rust handle, worker and native service has been destroyed here.
    // Platform has unmounted RomFS, so replacement cannot rename an open asset.
    if (pending_update) {
      char *error = nullptr;
      const bool installed = wodex_pending_install(&fs_api, pending_update.get(), &error);
      check_app(installed, error);
    }
    return 0;
  } catch (const std::exception &error) {
    std::cerr << "Wodex: " << error.what() << '\n';
#ifdef __SWITCH__
    consoleInit(nullptr);
    std::cout << "Wodex could not start.\n" << error.what() << "\nPress + to exit.\n";
    PadState pad;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
    while (appletMainLoop()) {
      padUpdate(&pad);
      if (padGetButtonsDown(&pad) & HidNpadButton_Plus)
        break;
      consoleUpdate(nullptr);
    }
    consoleExit(nullptr);
#endif
    return 1;
  }
}
