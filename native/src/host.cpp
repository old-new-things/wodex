#include "wodex_host.h"
#include "platform.hpp"
#include <SDL.h>
#include <algorithm>
#include <cstdlib>
#include <deque>
#include <stdexcept>
#include <utility>
#include <vector>
#ifdef __SWITCH__
#include <switch.h>
#endif

namespace wodex {
namespace {
WodexHostBytes borrowed(const std::string &value) noexcept {
  return {reinterpret_cast<const uint8_t *>(value.data()), value.size()};
}
std::string copied(WodexHostBytes value) {
  if (value.len && !value.data)
    throw std::runtime_error("Invalid native service input.");
  return value.len ? std::string(reinterpret_cast<const char *>(value.data), value.len) : std::string{};
}
uint32_t normalized_key(SDL_Keycode key) noexcept {
  switch (key) {
  case SDLK_ESCAPE: return 1;
  case SDLK_RETURN: case SDLK_KP_ENTER: return 2;
  case SDLK_SPACE: return 3;
  case SDLK_LEFT: return 4;
  case SDLK_RIGHT: return 5;
  case SDLK_UP: return 6;
  case SDLK_DOWN: return 7;
  case SDLK_BACKSPACE: return 8;
  case SDLK_DELETE: return 9;
  case SDLK_HOME: return 10;
  case SDLK_END: return 11;
  case SDLK_PAGEUP: return 12;
  case SDLK_PAGEDOWN: return 13;
  case SDLK_F1: return 14;
  case SDLK_MINUS: return 15;
  case SDLK_x: return 16;
  case SDLK_y: return 17;
  case SDLK_a: return 18;
  case SDLK_v: return 19;
  case SDLK_u: return 20;
  case SDLK_LEFTBRACKET: return 21;
  case SDLK_RIGHTBRACKET: return 22;
  default: return 0;
  }
}
uint32_t normalized_modifiers(Uint16 modifiers) noexcept {
  return (modifiers & KMOD_SHIFT ? 1u : 0u) | (modifiers & KMOD_CTRL ? 2u : 0u) |
         (modifiers & KMOD_GUI ? 4u : 0u);
}
} // namespace

struct NativeBindings::Impl {
  Platform &platform;
  std::string output;
  std::string error;
  const char *fallback_error = nullptr;
  std::deque<WodexHostInput> input_queue;
  bool input_failed = false;
  std::vector<SDL_GameController *> controllers;
#ifdef __SWITCH__
  PadState pad{};
#endif

  explicit Impl(Platform &platform_value) : platform(platform_value) {
#ifndef __SWITCH__
    try {
      for (int index = 0; index < SDL_NumJoysticks(); ++index)
        open_controller(index);
    } catch (...) {
      for (auto *controller : controllers)
        SDL_GameControllerClose(controller);
      throw;
    }
#else
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
#endif
  }

  ~Impl() {
    SDL_StopTextInput();
    for (auto *controller : controllers)
      SDL_GameControllerClose(controller);
  }

  template <typename Function> static bool guarded(void *context, Function action) noexcept {
    auto &self = *static_cast<Impl *>(context);
    self.error.clear();
    self.fallback_error = nullptr;
    try {
      action(self);
      return true;
    } catch (const std::exception &failure) {
      try {
        self.error = failure.what();
      } catch (...) {
        self.fallback_error = "Native service failed.";
      }
    } catch (...) {
      self.fallback_error = "Native service failed.";
    }
    return false;
  }

  static WodexHostBytes last_error(void *context) noexcept {
    auto &self = *static_cast<Impl *>(context);
    if (self.fallback_error) {
      const auto length = std::char_traits<char>::length(self.fallback_error);
      return {reinterpret_cast<const uint8_t *>(self.fallback_error), length};
    }
    return borrowed(self.error);
  }

  void open_controller(int index) {
#ifndef __SWITCH__
    if (!SDL_IsGameController(index))
      return;
    const auto instance = SDL_JoystickGetDeviceInstanceID(index);
    for (auto *controller : controllers) {
      if (SDL_JoystickInstanceID(SDL_GameControllerGetJoystick(controller)) == instance)
        return;
    }
    auto *controller = SDL_GameControllerOpen(index);
    if (controller) {
      try {
        controllers.push_back(controller);
      } catch (...) {
        SDL_GameControllerClose(controller);
        throw;
      }
    }
#else
    static_cast<void>(index);
#endif
  }

  void remove_controller(SDL_JoystickID instance) {
    const auto iterator = std::find_if(controllers.begin(), controllers.end(), [&](auto *controller) {
      return SDL_JoystickInstanceID(SDL_GameControllerGetJoystick(controller)) == instance;
    });
    if (iterator != controllers.end()) {
      SDL_GameControllerClose(*iterator);
      controllers.erase(iterator);
    }
  }

  void begin_frame() {
#ifdef __SWITCH__
    padUpdate(&pad);
    const u64 pressed = padGetButtonsDown(&pad);
    if (pressed & HidNpadButton_Plus)
      input_queue.push_back({1, 0, 0, 0, 0, 0, {nullptr, 0}});
    const auto button = [&](u64 mask, uint8_t value) {
      if (pressed & mask)
        input_queue.push_back({7, 0, 0, 0, 0, value, {nullptr, 0}});
    };
    button(HidNpadButton_Minus, SDL_CONTROLLER_BUTTON_BACK);
    button(HidNpadButton_A, SDL_CONTROLLER_BUTTON_A);
    button(HidNpadButton_B, SDL_CONTROLLER_BUTTON_B);
    button(HidNpadButton_X, SDL_CONTROLLER_BUTTON_X);
    button(HidNpadButton_Y, SDL_CONTROLLER_BUTTON_Y);
    button(HidNpadButton_Left, SDL_CONTROLLER_BUTTON_DPAD_LEFT);
    button(HidNpadButton_Right, SDL_CONTROLLER_BUTTON_DPAD_RIGHT);
    button(HidNpadButton_Up, SDL_CONTROLLER_BUTTON_DPAD_UP);
    button(HidNpadButton_Down, SDL_CONTROLLER_BUTTON_DPAD_DOWN);
    button(HidNpadButton_L, SDL_CONTROLLER_BUTTON_LEFTSHOULDER);
    button(HidNpadButton_R, SDL_CONTROLLER_BUTTON_RIGHTSHOULDER);
    const u64 held = padGetButtons(&pad);
    int scroll = (held & HidNpadButton_ZL ? -12 : 0) + (held & HidNpadButton_ZR ? 12 : 0);
    const auto stick = padGetStickPos(&pad, 1);
    if (std::abs(stick.y) > 8000)
      scroll -= stick.y / 2500;
    if (scroll)
      input_queue.push_back({6, 0, 0, 0, 0, scroll, {nullptr, 0}});
#endif
  }

  WodexHostInput poll_input() {
    if (input_failed)
      return {1, 0, 0, 0, 0, 0, {nullptr, 0}};
    if (!input_queue.empty()) {
      const auto result = input_queue.front();
      input_queue.pop_front();
      return result;
    }
    SDL_Event event;
    while (SDL_PollEvent(&event)) {
      WodexHostInput result{};
      switch (event.type) {
      case SDL_QUIT:
        result.kind = 1;
        return result;
      case SDL_KEYDOWN:
        result.key = normalized_key(event.key.keysym.sym);
        if (!result.key)
          continue;
        result.kind = 2;
        result.modifiers = normalized_modifiers(event.key.keysym.mod);
        return result;
      case SDL_TEXTINPUT:
        output = event.text.text;
        result.kind = 3;
        result.text = borrowed(output);
        return result;
      case SDL_TEXTEDITING:
        output = event.edit.text;
        result.kind = 4;
        result.text = borrowed(output);
        return result;
      case SDL_MOUSEBUTTONDOWN:
        if (event.button.button != SDL_BUTTON_LEFT)
          continue;
        result.kind = 5;
        // SDL's logical-size filter already scales mouse/touch coordinates.
        result.x = event.button.x;
        result.y = event.button.y;
        return result;
      case SDL_FINGERUP:
        result.kind = 5;
        result.x = static_cast<int32_t>(event.tfinger.x * 1280);
        result.y = static_cast<int32_t>(event.tfinger.y * 720);
        return result;
      case SDL_MOUSEWHEEL:
        result.kind = 6;
        result.value = -event.wheel.y * 80;
        return result;
#ifndef __SWITCH__
      case SDL_CONTROLLERDEVICEADDED:
        open_controller(event.cdevice.which);
        continue;
      case SDL_CONTROLLERDEVICEREMOVED:
        remove_controller(event.cdevice.which);
        continue;
      case SDL_CONTROLLERBUTTONDOWN:
        result.kind = 7;
        result.value = event.cbutton.button;
        return result;
#endif
      default:
        continue;
      }
    }
    return {};
  }

  WodexHostApi api() {
    WodexHostApi result{};
    result.context = this;
    result.now_ms = [](void *context) noexcept -> uint64_t {
      uint64_t value = 0;
      guarded(context, [&](Impl &) { value = SDL_GetTicks64(); });
      return value;
    };
    result.running = [](void *context) noexcept {
      bool value = false;
      guarded(context, [&](Impl &self) { value = self.platform.running(); });
      return value;
    };
    result.begin_frame = [](void *context) noexcept {
      // A native input allocation failure exits through the normal Rust close path.
      if (!guarded(context, [](Impl &self) { self.begin_frame(); }))
        static_cast<Impl *>(context)->input_failed = true;
    };
    result.poll_input = [](void *context) noexcept {
      WodexHostInput value{};
      if (!guarded(context, [&](Impl &self) { value = self.poll_input(); }))
        value.kind = 1;
      return value;
    };
    result.text_input = [](void *context, bool enabled) noexcept {
      guarded(context, [&](Impl &) {
        if (enabled) {
          SDL_Rect rect{390, 220, 800, 360};
          SDL_SetTextInputRect(&rect);
          SDL_StartTextInput();
        } else {
          SDL_StopTextInput();
        }
      });
    };
    result.clipboard = [](void *context) noexcept {
      WodexHostBytes value{};
      guarded(context, [&](Impl &self) {
        std::unique_ptr<char, decltype(&SDL_free)> text(SDL_GetClipboardText(), SDL_free);
        self.output = text ? text.get() : "";
        value = borrowed(self.output);
      });
      return value;
    };
    result.edit_system = [](void *context, WodexHostBytes title, WodexHostBytes initial, bool secret,
                            WodexHostBytes *output_value) noexcept {
      return guarded(context, [&](Impl &self) {
        self.output = self.platform.edit_text(copied(title), copied(initial), secret);
        *output_value = borrowed(self.output);
      });
    };
    result.last_error = last_error;
    return result;
  }
};

NativeBindings::NativeBindings(Platform &platform) : impl_(std::make_unique<Impl>(platform)) {}
NativeBindings::~NativeBindings() = default;
WodexHostApi NativeBindings::api() { return impl_->api(); }
} // namespace wodex
