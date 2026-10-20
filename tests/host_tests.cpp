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
WodexHostBytes bytes(const std::string &value) {
  return {reinterpret_cast<const uint8_t *>(value.data()), value.size()};
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

void test_storage(WodexHostApi &api, const std::string &path) {
  WodexHostBytes loaded{};
  check(api.load(api.context, &loaded) && loaded.len == 0, "Missing storage was not empty.");
  const std::string binary("abc\0\xffxyz", 8);
  check(api.save(api.context, bytes(binary)), "Host save failed.");
  check(api.load(api.context, &loaded), "Host load failed.");
  check(copied(loaded) == binary, "Native host altered non-UTF-8 or NUL storage bytes.");
  check(api.preserve(api.context, bytes(binary)), "Host recovery failed.");
  check(wodex::load_file(path + ".recovery-1") == binary, "Recovery altered corrupt data bytes.");
  const std::string later = "second recovery";
  check(api.preserve(api.context, bytes(later)), "Repeated host recovery failed.");
  check(wodex::load_file(path + ".recovery-1") == binary &&
        wodex::load_file(path + ".recovery-2") == later,
        "Host recovery replaced an earlier copy.");
  check(!api.save(api.context, {nullptr, 1}), "Invalid host buffer did not report failure.");
  check(!copied(api.last_error(api.context)).empty(), "Native exception had no diagnostic.");
  check(wodex::load_file(path) == binary, "Failed host write damaged the existing save.");
}

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
  check(!api.is_switch(api.context), "Desktop host reported a Switch platform.");
  check(!api.network_active(api.context), "New host reported an active network request.");
  check(api.poll_transport(api.context).kind == 0, "New host had a transport event.");
  check(!api.update_supported(api.context), "Desktop host allowed executable replacement.");
  check(api.update_status(api.context).phase == 0, "New updater was not idle.");
  bool ready = true;
  check(api.prepare_install(api.context, &ready) && !ready,
        "Host prepared an installation without a supported target.");
  check(api.update_download(api.context), "Updater callback failed unexpectedly.");
  const auto status = api.update_status(api.context);
  check(status.phase == 7 && !copied(status.message).empty(),
        "Unchecked download did not produce a useful updater error.");
  check(SDL_setenv("DEEPSEEK_API_KEY", "host-environment-test", 1) == 0,
        "Cannot set test environment key.");
  check(copied(api.environment_key(api.context)) == "host-environment-test",
        "Environment key was not exposed to the Rust application.");
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
    wodex::NativeBackend backend(platform, "0.2.0", "");
    auto api = backend.api();
    test_storage(api, platform.data_dir() + "/state.json");
    test_input(api);
    test_services(api);
    std::cout << "Native host: " << passed << " checks passed.\n";
    return 0;
  } catch (const std::exception &error) {
    std::cerr << "Native host failed: " << error.what() << '\n';
    return 1;
  }
}
