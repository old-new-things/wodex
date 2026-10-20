#include "storage.hpp"
#include "wodex_backend.h"
#include "wodex_core.h"

#include <memory>
#include <stdexcept>

namespace wodex {
namespace {

using RustString = std::unique_ptr<char, decltype(&wodex_string_free)>;
using RustBuffer = std::unique_ptr<WodexBuffer, decltype(&wodex_buffer_free)>;

[[noreturn]] void storage_error(char* raw_error) {
  RustString error{raw_error, &wodex_string_free};
  throw std::runtime_error(error ? error.get() : "The Rust storage backend failed.");
}

const char* native_path(const std::string& path) {
  // C strings cannot represent embedded NUL. Reject a lossy ABI conversion.
  if (path.find('\0') != std::string::npos) {
    throw std::runtime_error("Cannot pass a path containing NUL through the native ABI.");
  }
  return path.c_str();
}

}  // namespace

std::string load_file(const std::string& path) {
  const auto filesystem = wodex_native_fs_api();
  char* raw_error = nullptr;
  RustBuffer buffer{wodex_storage_load(&filesystem, native_path(path), &raw_error), &wodex_buffer_free};
  if (!buffer) storage_error(raw_error);
  const auto length = wodex_buffer_len(buffer.get());
  return length == 0 ? std::string{} : std::string{
    reinterpret_cast<const char*>(wodex_buffer_data(buffer.get())), length};
}

void atomic_write(const std::string& path, const std::string& content) {
  const auto filesystem = wodex_native_fs_api();
  char* raw_error = nullptr;
  if (!wodex_storage_write(&filesystem, native_path(path),
      reinterpret_cast<const uint8_t*>(content.data()), content.size(), &raw_error)) {
    storage_error(raw_error);
  }
}

std::string preserve_file(const std::string& path, const std::string& content) {
  const auto filesystem = wodex_native_fs_api();
  char* raw_error = nullptr;
  RustString output{wodex_storage_preserve(&filesystem, native_path(path),
    reinterpret_cast<const uint8_t*>(content.data()), content.size(), &raw_error), &wodex_string_free};
  if (!output) storage_error(raw_error);
  return output.get();
}

}  // namespace wodex
