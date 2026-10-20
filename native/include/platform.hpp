#pragma once

#include <cstddef>
#include <string>

namespace wodex {

// Keep Platform alive until after all fonts and network transports are destroyed.
class Platform {
public:
  Platform();
  ~Platform();
  Platform(const Platform&) = delete;
  Platform& operator=(const Platform&) = delete;

  std::string data_dir() const;
  std::string font_path() const;
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
  void* font_data_ = nullptr;
  std::size_t font_size_ = 0;
#ifdef __SWITCH__
  bool sockets_ready_ = false;
  bool network_ready_ = false;
  bool fonts_ready_ = false;
  bool romfs_ready_ = false;
#endif
};

// Missing files return an empty string. Other failures throw std::runtime_error.
// A bounded reader prevents malformed local data from exhausting applet memory.
std::string load_file(const std::string& path);

// Write a private temporary file, flush it, then replace the destination.
// An interrupted write cannot truncate the previous saved conversation.
void atomic_write(const std::string& path, const std::string& content);

// Preserve unreadable data without replacing an earlier recovery file.
// Returns the new path (path + ".recovery-N"). The source path is untouched.
std::string preserve_file(const std::string& path, const std::string& content);

}  // namespace wodex
