#pragma once

#include <string>

namespace wodex {

// Test-only C++ facades for the Rust backend. Binary data is passed with an
// explicit length; storage decisions are made exclusively by Rust.
std::string load_file(const std::string& path);
void atomic_write(const std::string& path, const std::string& content);
std::string preserve_file(const std::string& path, const std::string& content);

}  // namespace wodex
