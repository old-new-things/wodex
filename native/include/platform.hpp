#pragma once

#include <cstddef>
#include <string>

namespace wodex {

// Keep Platform alive until after all fonts and network transports are destroyed.
class Platform {
public:
  // Desktop resources live beside the executable. Switch uses its mounted RomFS.
  explicit Platform(const std::string& resource_dir = "");
  ~Platform();
  Platform(const Platform&) = delete;
  Platform& operator=(const Platform&) = delete;

  std::string data_dir() const;
  std::string font_path() const;
  std::string monospace_font_path() const;
  void* font_data() const;
  std::size_t font_size() const;
  bool running() const;

  // Switch uses the system keyboard. Desktop input is handled by the SDL UI.
  // Cancelling the keyboard preserves the original text.
  std::string edit_text(const std::string& title,
                       const std::string& initial,
                       bool secret = false);

private:
  void shutdown() noexcept;
  std::string data_dir_;
  std::string font_path_;
  std::string monospace_font_path_;
  void* font_data_ = nullptr;
  std::size_t font_size_ = 0;
#ifdef __SWITCH__
  bool sockets_ready_ = false;
  bool network_ready_ = false;
  bool fonts_ready_ = false;
  bool romfs_ready_ = false;
#endif
};

}  // namespace wodex
