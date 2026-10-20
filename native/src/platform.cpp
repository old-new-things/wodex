#include "platform.hpp"
#include "wodex_backend.h"
#include "wodex_core.h"

#include <array>
#include <memory>
#include <stdexcept>

#ifdef __SWITCH__
#include <switch.h>
#endif

namespace wodex {
Platform::Platform(const std::string& resource_dir) {
  try {
#ifdef __SWITCH__
    if (R_FAILED(socketInitializeDefault())) throw std::runtime_error("Cannot initialize Switch sockets.");
    sockets_ready_ = true;
    if (R_FAILED(nifmInitialize(NifmServiceType_User))) throw std::runtime_error("Cannot initialize Switch networking.");
    network_ready_ = true;
    if (R_FAILED(romfsInit())) throw std::runtime_error("Cannot mount application assets.");
    romfs_ready_ = true;
#endif
    const WodexFsApi filesystem = wodex_native_fs_api();
    char* raw_error = nullptr;
    using RustString = std::unique_ptr<char, decltype(&wodex_string_free)>;
    RustString selected_font{
      wodex_storage_font_path(&filesystem, resource_dir.c_str(), &raw_error), &wodex_string_free};
    RustString font_error{raw_error, &wodex_string_free};
    if (!selected_font) {
      throw std::runtime_error(font_error ? font_error.get() : "Cannot select application font.");
    }
    font_path_ = selected_font.get();
#ifdef __SWITCH__
    if (font_path_.empty()) {
      if (R_FAILED(plInitialize(PlServiceType_User))) throw std::runtime_error("Cannot initialize Switch fonts.");
      fonts_ready_ = true;
      PlFontData font{};
      bool selected_font = false;
      if (R_SUCCEEDED(setInitialize())) {
        u64 language = 0;
        const Result language_result = setGetSystemLanguage(&language);
        setExit();
        if (R_SUCCEEDED(language_result)) {
          std::array<PlFontData, PlSharedFontType_Total> fonts{};
          s32 count = 0;
          if (R_SUCCEEDED(plGetSharedFont(language, fonts.data(),
              static_cast<s32>(fonts.size()), &count)) && count > 0) {
            // Prioritize the console language when the bundled font is absent.
            font = fonts.front();
            selected_font = font.address != nullptr && font.size > 0;
          }
        }
      }
      if (!selected_font && R_FAILED(plGetSharedFontByType(&font, PlSharedFontType_Standard))) {
        throw std::runtime_error("Cannot load the Switch system font.");
      }
      font_data_ = font.address;
      font_size_ = font.size;
    }
#endif
    raw_error = nullptr;
    RustString directory{wodex_storage_data_dir(&filesystem, &raw_error), &wodex_string_free};
    RustString error{raw_error, &wodex_string_free};
    if (!directory) {
      throw std::runtime_error(error ? error.get() : "Cannot prepare saved data directory.");
    }
    data_dir_ = directory.get();
  } catch (...) {
    shutdown();
    throw;
  }
}

Platform::~Platform() { shutdown(); }

void Platform::shutdown() noexcept {
#ifdef __SWITCH__
  if (fonts_ready_) { plExit(); fonts_ready_ = false; }
  if (romfs_ready_) { romfsExit(); romfs_ready_ = false; }
  if (network_ready_) { nifmExit(); network_ready_ = false; }
  if (sockets_ready_) { socketExit(); sockets_ready_ = false; }
#endif
  font_data_ = nullptr;
  font_size_ = 0;
}

std::string Platform::data_dir() const { return data_dir_; }
std::string Platform::font_path() const { return font_path_; }
void* Platform::font_data() const { return font_data_; }
std::size_t Platform::font_size() const { return font_size_; }

bool Platform::running() const {
#ifdef __SWITCH__
  return appletMainLoop();
#else
  return true;
#endif
}

std::string Platform::edit_text(const std::string& title, const std::string& initial, bool secret) {
#ifdef __SWITCH__
  SwkbdConfig keyboard{};
  if (R_FAILED(swkbdCreate(&keyboard, 0))) throw std::runtime_error("Cannot open the system keyboard.");
  if (secret) swkbdConfigMakePresetPassword(&keyboard);
  else {
    swkbdConfigMakePresetDefault(&keyboard);
    swkbdConfigSetType(&keyboard, SwkbdType_All);
  }
  swkbdConfigSetHeaderText(&keyboard, title.c_str());
  swkbdConfigSetGuideText(&keyboard, title.c_str());
  swkbdConfigSetInitialText(&keyboard, initial.c_str());
  swkbdConfigSetStringLenMax(&keyboard, secret ? 249 : 1000);
  swkbdConfigSetReturnButtonFlag(&keyboard, secret ? 0 : 1);
  swkbdConfigSetDicFlag(&keyboard, secret ? 0 : 1);
  std::array<char, 16385> result{};
  const Result status = swkbdShow(&keyboard, result.data(), result.size());
  if (secret) {
    volatile u8* bytes = keyboard.workbuf;
    for (std::size_t index = 0; index < keyboard.workbuf_size; ++index) bytes[index] = 0;
  }
  swkbdClose(&keyboard);
  std::string text = R_FAILED(status) ? initial : std::string{result.data()};
  // The app retains the final string; avoid retaining a second key copy on stack.
  if (secret) {
    volatile char* bytes = result.data();
    for (std::size_t index = 0; index < result.size(); ++index) bytes[index] = 0;
  }
  return text;
#else
  static_cast<void>(title);
  static_cast<void>(secret);
  return initial;
#endif
}

}  // namespace wodex
