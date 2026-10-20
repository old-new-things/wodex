#include "platform.hpp"
#include "wodex_host.h"
#include <SDL.h>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <stdexcept>
#include <string>
#include <unistd.h>

namespace {
int passed = 0;
void check(bool condition, const char *description) {
  if (!condition)
    throw std::runtime_error(description);
  ++passed;
}
std::string copied(WodexHostBytes value) {
  return value.len ? std::string(reinterpret_cast<const char *>(value.data), value.len) : "";
}
class SdlRuntime {
public:
  SdlRuntime() {
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMECONTROLLER) != 0)
      throw std::runtime_error(SDL_GetError());
  }
  ~SdlRuntime() { SDL_Quit(); }
};
struct TemporaryStorage {
  std::filesystem::path root;
  TemporaryStorage() {
    char pattern[] = "/tmp/wodex-host-test-XXXXXX";
    char *directory = ::mkdtemp(pattern);
    if (!directory)
      throw std::runtime_error("Cannot create temporary host storage.");
    root = directory;
    if (SDL_setenv("WODEX_DATA_DIR", directory, 1) != 0)
      throw std::runtime_error("Cannot select temporary host storage.");
  }
  ~TemporaryStorage() {
    std::error_code error;
    std::filesystem::remove_all(root, error);
  }
};

void test_input(WodexHostApi &api) {
  api.text_input(api.context, true);
  SDL_FlushEvents(SDL_FIRSTEVENT, SDL_LASTEVENT);
  // Queue a whole frame before polling; SDL_PollEvent inserts a frame sentinel.
  // Events arriving after the sentinel are delivered on the following frame.
  SDL_Event event{};
  event.type = SDL_KEYDOWN;
  event.key.keysym.sym = SDLK_LEFT;
  event.key.keysym.mod = KMOD_CTRL | KMOD_SHIFT;
  check(SDL_PushEvent(&event) == 1, "Cannot queue key input.");
  event = {};
  event.type = SDL_TEXTINPUT;
  std::strcpy(event.text.text, "你好");
  check(SDL_PushEvent(&event) == 1, "Cannot queue UTF-8 input.");
  event = {};
  event.type = SDL_TEXTEDITING;
  std::strcpy(event.edit.text, "中");
  check(SDL_PushEvent(&event) == 1, "Cannot queue IME composition.");
  event = {};
  event.type = SDL_MOUSEBUTTONDOWN;
  event.button.button = SDL_BUTTON_LEFT;
  event.button.x = 810;
  event.button.y = 620;
  check(SDL_PushEvent(&event) == 1, "Cannot queue pointer input.");
  event = {};
  event.type = SDL_MOUSEWHEEL;
  event.wheel.y = 2;
  check(SDL_PushEvent(&event) == 1, "Cannot queue scrolling.");
  event = {};
  event.type = SDL_CONTROLLERBUTTONDOWN;
  event.cbutton.button = SDL_CONTROLLER_BUTTON_X;
  check(SDL_PushEvent(&event) == 1, "Cannot queue controller input.");
  event = {};
  event.type = SDL_QUIT;
  check(SDL_PushEvent(&event) == 1, "Cannot queue quit input.");

  auto input = api.poll_input(api.context);
  check(input.kind == 2 && input.key == 4 && input.modifiers == 3,
        "Arrow or modifier normalization changed.");
  input = api.poll_input(api.context);
  check(input.kind == 3 && copied(input.text) == "你好", "UTF-8 text input was altered.");
  input = api.poll_input(api.context);
  check(input.kind == 4 && copied(input.text) == "中", "IME composition was altered.");
  input = api.poll_input(api.context);
  check(input.kind == 5 && input.x == 810 && input.y == 620,
        "Pointer coordinates were rescaled or altered.");
  input = api.poll_input(api.context);
  check(input.kind == 6 && input.value == -160, "Scroll direction or distance changed.");
  input = api.poll_input(api.context);
  check(input.kind == 7 && input.value == SDL_CONTROLLER_BUTTON_X,
        "Controller button normalization changed.");
  check(api.poll_input(api.context).kind == 1, "Quit input was lost.");
  check(api.poll_input(api.context).kind == 0, "Host invented input after the event queue drained.");
  api.text_input(api.context, false);
}

void test_services(WodexHostApi &api) {
  check(api.running(api.context), "Desktop platform was not running.");
  check(SDL_SetClipboardText("粘贴 text") == 0, "Cannot set the test clipboard.");
  check(copied(api.clipboard(api.context)) == "粘贴 text", "Clipboard UTF-8 text was altered.");
}
} // namespace

int main(int argc, char **argv) {
  try {
    if (argc != 2)
      throw std::runtime_error("Provide the desktop resource directory.");
    SdlRuntime sdl;
    TemporaryStorage storage;
    wodex::Platform platform(argv[1]);
    wodex::NativeBindings bindings(platform);
    auto api = bindings.api();
    test_input(api);
    test_services(api);
    std::cout << "Native host: " << passed << " checks passed.\n";
    return 0;
  } catch (const std::exception &error) {
    std::cerr << "Native host failed: " << error.what() << '\n';
    return 1;
  }
}
