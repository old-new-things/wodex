#include "platform.hpp"

#include <array>
#include <atomic>
#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <stdexcept>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#ifdef __SWITCH__
#include <switch.h>
#endif

namespace wodex {
namespace {

constexpr std::size_t max_saved_file_size = 32 * 1024 * 1024;
std::atomic<unsigned long> temp_counter{0};

#ifndef __SWITCH__
std::string environment(const char* name) {
  const char* value = std::getenv(name);
  return value == nullptr ? std::string{} : std::string{value};
}
#endif

[[noreturn]] void file_error(const char* operation, int error = errno) {
  throw std::runtime_error(std::string{operation} + ": " + std::strerror(error));
}

class FileDescriptor {
public:
  explicit FileDescriptor(int value) : value_(value) {}
  ~FileDescriptor() { if (value_ >= 0) ::close(value_); }
  int get() const { return value_; }
  int close() {
    const int value = value_;
    value_ = -1;
    return ::close(value);
  }
  FileDescriptor(const FileDescriptor&) = delete;
  FileDescriptor& operator=(const FileDescriptor&) = delete;

private:
  int value_;
};

void write_durable_file(FileDescriptor& file, const std::string& content) {
#ifndef __SWITCH__
  if (::fchmod(file.get(), 0600) != 0) file_error("Cannot protect save file");
#endif
  std::size_t written = 0;
  while (written < content.size()) {
    const ssize_t count = ::write(file.get(), content.data() + written, content.size() - written);
    if (count < 0) {
      if (errno == EINTR) continue;
      file_error("Cannot write saved data");
    }
    if (count == 0) throw std::runtime_error("Cannot finish writing saved data.");
    written += static_cast<std::size_t>(count);
  }
  if (::fsync(file.get()) != 0) file_error("Cannot flush saved data");
  if (file.close() != 0) file_error("Cannot close saved data");
}

#ifndef __SWITCH__
std::string parent_directory(const std::string& path) {
  const auto separator = path.find_last_of('/');
  if (separator == std::string::npos) return ".";
  if (separator == 0) return "/";
  return path.substr(0, separator);
}
#endif

void flush_parent_directory(const std::string& path) {
#ifndef __SWITCH__
  int flags = O_RDONLY;
#ifdef O_DIRECTORY
  flags |= O_DIRECTORY;
#endif
  FileDescriptor directory{::open(parent_directory(path).c_str(), flags)};
  if (directory.get() < 0) file_error("Cannot open save directory");
  if (::fsync(directory.get()) != 0) file_error("Cannot flush save directory");
#else
  // Horizon flushes file data via fsFileFlush; SD has no savedata commit step.
  static_cast<void>(path);
#endif
}

void create_directories(const std::string& path) {
  if (path.empty()) throw std::runtime_error("The data directory is empty.");
  std::size_t start = path.front() == '/' ? 1 : 0;
#ifdef __SWITCH__
  const auto device_end = path.find(":/");
  if (device_end != std::string::npos) start = device_end + 2;
#endif
  for (std::size_t end = start; end <= path.size(); ++end) {
    if (end != path.size() && path[end] != '/') continue;
    const std::string directory = path.substr(0, end);
    if (directory.empty()) continue;
    if (::mkdir(directory.c_str(), 0700) != 0 && errno != EEXIST) {
      file_error("Cannot create data directory");
    }
    struct stat status{};
    if (::stat(directory.c_str(), &status) != 0) file_error("Cannot inspect data directory");
    if (!S_ISDIR(status.st_mode)) throw std::runtime_error("The data path is not a directory.");
  }
#ifndef __SWITCH__
  // chmod only our final application directory, never shared parent directories.
  if (::chmod(path.c_str(), 0700) != 0) file_error("Cannot protect data directory");
#endif
}

bool readable_file(const std::string& path) {
  struct stat status{};
  return !path.empty() && ::stat(path.c_str(), &status) == 0 &&
    S_ISREG(status.st_mode) && ::access(path.c_str(), R_OK) == 0;
}

}  // namespace

Platform::Platform(const std::string& resource_dir) {
  try {
#ifdef __SWITCH__
    if (R_FAILED(socketInitializeDefault())) throw std::runtime_error("Cannot initialize Switch sockets.");
    sockets_ready_ = true;
    if (R_FAILED(nifmInitialize(NifmServiceType_User))) throw std::runtime_error("Cannot initialize Switch networking.");
    network_ready_ = true;
    if (R_FAILED(romfsInit())) throw std::runtime_error("Cannot mount application assets.");
    romfs_ready_ = true;
    static_cast<void>(resource_dir);
    const std::string bundled_font = "romfs:/fonts/NotoSansCJKsc-Regular.otf";
    if (readable_file(bundled_font)) {
      font_path_ = bundled_font;
    } else {
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
    data_dir_ = "sdmc:/switch/wodex";
#else
    data_dir_ = environment("WODEX_DATA_DIR");
    if (data_dir_.empty()) {
      const std::string user_home = environment("HOME");
#ifdef __APPLE__
      if (user_home.empty()) throw std::runtime_error("Set HOME or WODEX_DATA_DIR to save conversations.");
      data_dir_ = user_home + "/Library/Application Support/Wodex";
#else
      const std::string xdg_data = environment("XDG_DATA_HOME");
      if (!xdg_data.empty() && xdg_data.front() == '/') data_dir_ = xdg_data + "/wodex";
      else if (!user_home.empty()) data_dir_ = user_home + "/.local/share/wodex";
      else throw std::runtime_error("Set HOME or WODEX_DATA_DIR to save conversations.");
#endif
    }
    const std::string configured_font = environment("WODEX_FONT");
    if (!configured_font.empty()) {
      if (!readable_file(configured_font)) throw std::runtime_error("WODEX_FONT does not point to a readable font.");
      font_path_ = configured_font;
    } else if (!resource_dir.empty() && readable_file(resource_dir + "/fonts/NotoSansCJKsc-Regular.otf")) {
      font_path_ = resource_dir + "/fonts/NotoSansCJKsc-Regular.otf";
    } else {
      const std::array<const char*, 7> candidates = {
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"
      };
      for (const auto* candidate : candidates) {
        if (readable_file(candidate)) { font_path_ = candidate; break; }
      }
      if (font_path_.empty()) throw std::runtime_error("No font found. Set WODEX_FONT to a TrueType or OpenType font.");
    }
#endif
    create_directories(data_dir_);
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
  // libnx's foreground keyboard initializes at most 249 UTF-16 code units.
  // Refuse a lossy edit instead of silently replacing a long draft with a prefix.
  std::size_t initial_units = 0;
  for (const unsigned char byte : initial) {
    if ((byte & 0xc0) != 0x80) initial_units += byte >= 0xf0 ? 2 : 1;
  }
  if (initial_units > 249) {
    throw std::runtime_error("The system keyboard cannot reopen a draft longer than 249 characters. Send or clear this draft first.");
  }
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

std::string load_file(const std::string& path) {
  if (path.empty() || path.find('\0') != std::string::npos) throw std::runtime_error("Invalid saved data path.");
  int descriptor = ::open(path.c_str(), O_RDONLY);
#ifdef __SWITCH__
  // FAT/Horizon rename cannot replace an existing file. A .bak survives the
  // short rename gap if power is lost while atomic_write rotates the old save.
  if (descriptor < 0 && errno == ENOENT) descriptor = ::open((path + ".bak").c_str(), O_RDONLY);
#endif
  FileDescriptor file{descriptor};
  if (file.get() < 0) {
    if (errno == ENOENT) return {};
    file_error("Cannot open saved data");
  }
  struct stat status{};
  if (::fstat(file.get(), &status) != 0) file_error("Cannot inspect saved data");
  if (!S_ISREG(status.st_mode)) throw std::runtime_error("Saved data is not a regular file.");
  if (status.st_size < 0 || static_cast<unsigned long long>(status.st_size) > max_saved_file_size) {
    throw std::runtime_error("Saved data exceeds the 32 MiB size limit.");
  }
  std::string content;
  content.reserve(static_cast<std::size_t>(status.st_size));
  std::array<char, 16384> buffer{};
  while (true) {
    const ssize_t count = ::read(file.get(), buffer.data(), buffer.size());
    if (count == 0) break;
    if (count < 0) {
      if (errno == EINTR) continue;
      file_error("Cannot read saved data");
    }
    if (content.size() + static_cast<std::size_t>(count) > max_saved_file_size) {
      throw std::runtime_error("Saved data exceeds the 32 MiB size limit.");
    }
    content.append(buffer.data(), static_cast<std::size_t>(count));
  }
  return content;
}

void atomic_write(const std::string& path, const std::string& content) {
  if (path.empty() || path.find('\0') != std::string::npos) throw std::runtime_error("Invalid save path.");
  if (content.size() > max_saved_file_size) throw std::runtime_error("Saved data exceeds the 32 MiB size limit.");
  const std::string temporary = path + ".tmp." + std::to_string(::getpid()) + "." + std::to_string(temp_counter++);
  int flags = O_WRONLY | O_CREAT | O_EXCL;
#ifdef O_NOFOLLOW
  flags |= O_NOFOLLOW;
#endif
  FileDescriptor file{::open(temporary.c_str(), flags, 0600)};
  if (file.get() < 0) file_error("Cannot create temporary save file");
  try {
    write_durable_file(file, content);
    if (::rename(temporary.c_str(), path.c_str()) != 0) {
#ifdef __SWITCH__
      if (errno != EEXIST) file_error("Cannot replace saved data");
      const std::string backup = path + ".bak";
      if (::unlink(backup.c_str()) != 0 && errno != ENOENT) file_error("Cannot rotate previous save backup");
      if (::rename(path.c_str(), backup.c_str()) != 0) file_error("Cannot back up previous saved data");
      if (::rename(temporary.c_str(), path.c_str()) != 0) {
        const int error = errno;
        ::rename(backup.c_str(), path.c_str());
        file_error("Cannot replace saved data", error);
      }
#else
      file_error("Cannot replace saved data");
#endif
    }
    flush_parent_directory(path);
  } catch (...) {
    ::unlink(temporary.c_str());
    throw;
  }
}

std::string preserve_file(const std::string& path, const std::string& content) {
  if (path.empty() || path.find('\0') != std::string::npos) throw std::runtime_error("Invalid recovery path.");
  if (content.size() > max_saved_file_size) throw std::runtime_error("Saved data exceeds the 32 MiB size limit.");
  int flags = O_WRONLY | O_CREAT | O_EXCL;
#ifdef O_NOFOLLOW
  flags |= O_NOFOLLOW;
#endif
  for (unsigned int index = 1; index <= 9999; ++index) {
    const std::string recovery_path = path + ".recovery-" + std::to_string(index);
    FileDescriptor file{::open(recovery_path.c_str(), flags, 0600)};
    if (file.get() < 0) {
      if (errno == EEXIST) continue;
      file_error("Cannot create recovery file");
    }
    try {
      write_durable_file(file, content);
      flush_parent_directory(recovery_path);
      return recovery_path;
    } catch (...) {
      ::unlink(recovery_path.c_str());
      throw;
    }
  }
  throw std::runtime_error("The recovery file limit was reached. Move older recovery files before trying again.");
}

}  // namespace wodex
