#include "updater.hpp"
#include "wodex_core.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <fcntl.h>
#include <functional>
#include <limits>
#include <mutex>
#include <stdexcept>
#include <sys/stat.h>
#include <thread>
#include <unistd.h>
#include <utility>

#include <curl/curl.h>
#include <nlohmann/json.hpp>

namespace wodex {
namespace {

using Json = nlohmann::json;
constexpr std::uint64_t max_update_size = 128 * 1024 * 1024;
constexpr std::size_t max_release_size = 2 * 1024 * 1024;
using Easy = std::unique_ptr<CURL, decltype(&curl_easy_cleanup)>;
using Url = std::unique_ptr<CURLU, decltype(&curl_url_cleanup)>;
using CurlString = std::unique_ptr<char, decltype(&curl_free)>;
using RustString = std::unique_ptr<char, decltype(&wodex_string_free)>;
using Hash = std::unique_ptr<WodexSha256, decltype(&wodex_sha256_free)>;

class Cancelled : public std::exception {};

struct CurlGlobal {
  CurlGlobal() {
    if (curl_global_init(CURL_GLOBAL_DEFAULT) != CURLE_OK) throw std::runtime_error("Cannot initialize update networking.");
  }
  ~CurlGlobal() { curl_global_cleanup(); }
};

class File {
public:
  explicit File(int descriptor) : descriptor_(descriptor) {}
  ~File() { if (descriptor_ >= 0) ::close(descriptor_); }
  File(const File&) = delete;
  File& operator=(const File&) = delete;
  int get() const { return descriptor_; }
  void finish() {
    if (::fsync(descriptor_) != 0) throw std::runtime_error("Cannot flush the update file to storage.");
    const int descriptor = descriptor_;
    descriptor_ = -1;
    if (::close(descriptor) != 0) throw std::runtime_error("Cannot close the update file.");
  }
private:
  int descriptor_;
};

bool control_or_space(const std::string& text) {
  return std::any_of(text.begin(), text.end(), [](unsigned char byte) { return byte <= 0x20 || byte == 0x7f; });
}

bool ends_with(const std::string& text, const std::string& suffix) {
  return text.size() >= suffix.size() && text.compare(text.size() - suffix.size(), suffix.size(), suffix) == 0;
}

bool valid_repository(const std::string& repository) {
  const auto slash = repository.find('/');
  if (slash == 0 || slash == std::string::npos || slash > 39 || slash + 1 == repository.size() || repository.size() - slash - 1 > 100 ||
      repository.find('/', slash + 1) != std::string::npos || repository.find("..") != std::string::npos) return false;
  const auto alphanumeric = [](unsigned char byte) {
    return (byte >= 'a' && byte <= 'z') || (byte >= 'A' && byte <= 'Z') ||
      (byte >= '0' && byte <= '9');
  };
  if (!alphanumeric(repository.front()) || !alphanumeric(repository[slash - 1]) ||
      repository.substr(0, slash).find("--") != std::string::npos) return false;
  for (std::size_t index = 0; index < slash; ++index) {
    if (!alphanumeric(repository[index]) && repository[index] != '-') return false;
  }
  return std::all_of(repository.begin() + static_cast<std::ptrdiff_t>(slash + 1), repository.end(), [&](unsigned char byte) {
    return alphanumeric(byte) || byte == '-' || byte == '_' || byte == '.';
  });
}

std::string ascii_lower(std::string text) {
  std::transform(text.begin(), text.end(), text.begin(), [](unsigned char byte) {
    return byte >= 'A' && byte <= 'Z' ? static_cast<char>(byte + ('a' - 'A')) : static_cast<char>(byte);
  });
  return text;
}

bool url_has(CURLU* url, CURLUPart part) {
  char* raw = nullptr;
  const CURLUcode result = curl_url_get(url, part, &raw, 0);
  CurlString owned{raw, &curl_free};
  return result == CURLUE_OK;
}

std::string url_part(CURLU* url, CURLUPart part) {
  char* raw = nullptr;
  const CURLUcode result = curl_url_get(url, part, &raw, 0);
  CurlString owned{raw, &curl_free};
  return result == CURLUE_OK && raw ? std::string{raw} : std::string{};
}

Url parse_url(const std::string& text) {
  if (text.empty() || text.size() > 8192 || control_or_space(text) || text.find('\\') != std::string::npos) {
    throw std::runtime_error("The update server supplied an invalid URL.");
  }
  Url parsed{curl_url(), &curl_url_cleanup};
  if (!parsed || curl_url_set(parsed.get(), CURLUPART_URL, text.c_str(), 0) != CURLUE_OK ||
      url_has(parsed.get(), CURLUPART_USER) || url_has(parsed.get(), CURLUPART_PASSWORD) ||
      url_has(parsed.get(), CURLUPART_FRAGMENT)) throw std::runtime_error("The update URL contains unsupported credentials or fragments.");
  return parsed;
}

Url github_url(const std::string& text) {
  auto parsed = parse_url(text);
  const std::string host = url_part(parsed.get(), CURLUPART_HOST);
  const std::string port = url_part(parsed.get(), CURLUPART_PORT);
  const std::array<const char*, 5> hosts = {"api.github.com", "github.com", "release-assets.githubusercontent.com",
    "objects.githubusercontent.com", "github-releases.githubusercontent.com"};
  if (url_part(parsed.get(), CURLUPART_SCHEME) != "https" || (!port.empty() && port != "443") ||
      std::find(hosts.begin(), hosts.end(), host) == hosts.end()) {
    throw std::runtime_error("Updates may only use verified HTTPS GitHub release servers.");
  }
  return parsed;
}

std::string validate_test_base(const std::string& text) {
  if (text.empty()) return {};
  auto parsed = parse_url(text);
  const std::string host = url_part(parsed.get(), CURLUPART_HOST);
  const std::string path = url_part(parsed.get(), CURLUPART_PATH);
  if (url_part(parsed.get(), CURLUPART_SCHEME) != "http" ||
      (host != "127.0.0.1" && host != "localhost" && host != "[::1]") ||
      (path != "/" && !path.empty()) || url_has(parsed.get(), CURLUPART_QUERY)) {
    throw std::runtime_error("The update test server must be an explicit HTTP loopback origin.");
  }
  return ends_with(text, "/") ? text.substr(0, text.size() - 1) : text;
}

std::string request_url(const std::string& canonical, const std::string& test_base) {
  auto parsed = github_url(canonical);
  if (test_base.empty()) return canonical;
  std::string mapped = test_base + "/" + url_part(parsed.get(), CURLUPART_HOST) + url_part(parsed.get(), CURLUPART_PATH);
  const std::string query = url_part(parsed.get(), CURLUPART_QUERY);
  if (!query.empty()) mapped += "?" + query;
  return mapped;
}

std::string redirect_url(const std::string& current, const std::string& location) {
  auto parsed = github_url(current);
  if (curl_url_set(parsed.get(), CURLUPART_URL, location.c_str(), 0) != CURLUE_OK) {
    throw std::runtime_error("The update server returned an invalid redirect.");
  }
  const std::string next = url_part(parsed.get(), CURLUPART_URL);
  github_url(next);
  return next;
}

struct Transfer {
  CURL* easy;
  std::atomic<bool>* cancelled;
  std::function<void(const char*, std::size_t)> sink;
  std::exception_ptr failure;
  std::string location;
  std::size_t header_bytes = 0;
  std::uint64_t received = 0;
  std::uint64_t limit;
};

std::size_t receive_body(char* data, std::size_t size, std::size_t count, void* owner) noexcept {
  auto& transfer = *static_cast<Transfer*>(owner);
  try {
    if (size && count > std::numeric_limits<std::size_t>::max() / size) throw std::runtime_error("The update response is too large.");
    const std::size_t bytes = size * count;
    if (transfer.cancelled->load()) throw Cancelled{};
    long status = 0;
    curl_easy_getinfo(transfer.easy, CURLINFO_RESPONSE_CODE, &status);
    if (status != 200) return 0;
    if (bytes > transfer.limit - transfer.received) throw std::runtime_error("The update response exceeds its expected size.");
    transfer.sink(data, bytes);
    transfer.received += bytes;
    return bytes;
  } catch (...) {
    transfer.failure = std::current_exception();
    return 0;
  }
}

std::size_t receive_header(char* data, std::size_t size, std::size_t count, void* owner) noexcept {
  auto& transfer = *static_cast<Transfer*>(owner);
  try {
    if (size && count > std::numeric_limits<std::size_t>::max() / size) return 0;
    const std::size_t bytes = size * count;
    if (bytes > 65536 - transfer.header_bytes) throw std::runtime_error("The update response headers are too large.");
    transfer.header_bytes += bytes;
    std::string line{data, bytes};
    if (line.compare(0, 5, "HTTP/") == 0) transfer.location.clear();
    if (line.size() >= 9) {
      std::string name = line.substr(0, 9);
      std::transform(name.begin(), name.end(), name.begin(), [](unsigned char byte) {
        return byte >= 'A' && byte <= 'Z' ? static_cast<char>(byte + ('a' - 'A')) : static_cast<char>(byte);
      });
      if (name == "location:") {
        auto first = line.find_first_not_of(" \t", 9);
        auto last = line.find_last_not_of(" \t\r\n");
        transfer.location = first != std::string::npos && last >= first ? line.substr(first, last - first + 1) : "";
        if (transfer.location.size() > 8192) throw std::runtime_error("The update redirect is too long.");
      }
    }
    return bytes;
  } catch (...) {
    transfer.failure = std::current_exception();
    return 0;
  }
}

void fetch(std::string canonical, const std::string& test_base, std::atomic<bool>& cancelled,
           std::uint64_t limit, const std::function<void(const char*, std::size_t)>& sink) {
  for (int redirects = 0; redirects <= 5; ++redirects) {
    if (cancelled.load()) throw Cancelled{};
    const std::string actual = request_url(canonical, test_base);
    Easy easy{curl_easy_init(), &curl_easy_cleanup};
    if (!easy) throw std::runtime_error("Cannot create an update request.");
    Transfer transfer{easy.get(), &cancelled, sink, {}, {}, 0, 0, limit};
    const auto option = [&easy](CURLoption name, auto value) {
      if (curl_easy_setopt(easy.get(), name, value) != CURLE_OK) throw std::runtime_error("Cannot configure secure update networking.");
    };
    option(CURLOPT_URL, actual.c_str());
    option(CURLOPT_FOLLOWLOCATION, 0L);
#if LIBCURL_VERSION_NUM >= 0x075500
    option(CURLOPT_PROTOCOLS_STR, test_base.empty() ? "https" : "http");
#else
    option(CURLOPT_PROTOCOLS, static_cast<long>(test_base.empty() ? CURLPROTO_HTTPS : CURLPROTO_HTTP));
#endif
    option(CURLOPT_SSL_VERIFYPEER, 1L);
    option(CURLOPT_SSL_VERIFYHOST, 2L);
    option(CURLOPT_CONNECTTIMEOUT, 15L);
    option(CURLOPT_TIMEOUT, 600L);
    option(CURLOPT_LOW_SPEED_LIMIT, 1L);
    option(CURLOPT_LOW_SPEED_TIME, 90L);
    option(CURLOPT_NOSIGNAL, 1L);
    option(CURLOPT_NOPROGRESS, 0L);
    option(CURLOPT_USERAGENT, "Wodex-Updater/1");
    option(CURLOPT_WRITEFUNCTION, &receive_body);
    option(CURLOPT_WRITEDATA, &transfer);
    option(CURLOPT_HEADERFUNCTION, &receive_header);
    option(CURLOPT_HEADERDATA, &transfer);
    option(CURLOPT_XFERINFOFUNCTION, +[](void* owner, curl_off_t, curl_off_t, curl_off_t, curl_off_t) -> int {
      return static_cast<std::atomic<bool>*>(owner)->load() ? 1 : 0;
    });
    option(CURLOPT_XFERINFODATA, &cancelled);
    if (!test_base.empty()) option(CURLOPT_NOPROXY, "*");
#ifdef __SWITCH__
    option(CURLOPT_CAINFO, "romfs:/cacert.pem");
#else
    const char* ca_bundle = std::getenv("WODEX_CA_BUNDLE");
    if (ca_bundle && *ca_bundle) option(CURLOPT_CAINFO, ca_bundle);
#endif
    curl_slist* headers = nullptr;
    headers = curl_slist_append(headers, "Accept: application/vnd.github+json, application/octet-stream");
    if (!headers) throw std::runtime_error("Cannot allocate update request headers.");
    std::unique_ptr<curl_slist, decltype(&curl_slist_free_all)> owned_headers{headers, &curl_slist_free_all};
    option(CURLOPT_HTTPHEADER, headers);
    const CURLcode result = curl_easy_perform(easy.get());
    long status = 0;
    curl_easy_getinfo(easy.get(), CURLINFO_RESPONSE_CODE, &status);
    if (cancelled.load()) throw Cancelled{};
    if (transfer.failure) std::rethrow_exception(transfer.failure);
    if (status == 301 || status == 302 || status == 303 || status == 307 || status == 308) {
      if (redirects == 5 || transfer.location.empty()) throw std::runtime_error("The update server returned too many or invalid redirects.");
      canonical = redirect_url(canonical, transfer.location);
      continue;
    }
    if (status != 200 && status != 0) {
      if (status == 404) throw std::runtime_error("No published release was found. Check the configured public repository.");
      if (status == 403 || status == 429) throw std::runtime_error("GitHub's public update rate limit was reached. Try again later.");
      throw std::runtime_error("The update server returned HTTP " + std::to_string(status) + ". Try again later.");
    }
    if (result != CURLE_OK) {
      if (result == CURLE_PEER_FAILED_VERIFICATION) throw std::runtime_error("The update server's TLS certificate could not be verified.");
      throw std::runtime_error("The update download failed. Check your network and storage, then try again.");
    }
    if (status != 200) throw std::runtime_error("The update server did not return a release.");
    return;
  }
}

std::string backup_path(const std::string& target) { return target.substr(0, target.size() - 4) + ".previous.nro"; }

void validate_target_path(const std::string& target) {
  std::size_t start = 1;
  if (target.compare(0, 6, "sdmc:/") == 0) start = 6;
  else if (target.empty() || target.front() != '/') throw std::runtime_error("The running NRO path is unknown. Install updates manually.");
  if (target.size() > 4096 || !ends_with(target, ".nro") || ends_with(target, ".previous.nro") ||
      target.find('\0') != std::string::npos || target.find('\\') != std::string::npos ||
      target.find(':', start) != std::string::npos || std::any_of(target.begin(), target.end(), [](unsigned char byte) { return byte < 0x20 || byte == 0x7f; })) {
    throw std::runtime_error("The running NRO path cannot be safely replaced.");
  }
  while (start < target.size()) {
    const auto end = target.find('/', start);
    const auto component = target.substr(start, end == std::string::npos ? std::string::npos : end - start);
    if (component.empty() || component == "." || component == "..") throw std::runtime_error("The update path contains unsupported traversal.");
    if (end == std::string::npos) break;
    start = end + 1;
  }
}

bool file_status(const std::string& path, struct stat& status) {
#ifdef __SWITCH__
  const int result = ::stat(path.c_str(), &status);
#else
  const int result = ::lstat(path.c_str(), &status);
#endif
  if (result == 0) return true;
  if (errno == ENOENT) return false;
  throw std::runtime_error("Cannot inspect update files on storage.");
}

int open_read(const std::string& path) {
  int flags = O_RDONLY;
#ifdef O_NOFOLLOW
  flags |= O_NOFOLLOW;
#endif
  const int descriptor = ::open(path.c_str(), flags);
  if (descriptor < 0) throw std::runtime_error("Cannot open the NRO file on storage.");
  return descriptor;
}

std::uint32_t little32(const std::uint8_t* data) {
  std::uint32_t value = 0;
  for (int index = 3; index >= 0; --index) value = (value << 8) | data[index];
  return value;
}

std::uint64_t little64(const std::uint8_t* data) {
  std::uint64_t value = 0;
  for (int index = 7; index >= 0; --index) value = (value << 8) | data[index];
  return value;
}

void read_exact(int descriptor, void* buffer, std::size_t size) {
  auto* bytes = static_cast<char*>(buffer);
  std::size_t offset = 0;
  while (offset < size) {
    const ssize_t count = ::read(descriptor, bytes + offset, size - offset);
    if (count < 0 && errno == EINTR) continue;
    if (count <= 0) throw std::runtime_error("Cannot read the complete NRO header.");
    offset += static_cast<std::size_t>(count);
  }
}

void validate_romfs(int descriptor, std::uint64_t offset, std::uint64_t size) {
  constexpr std::uint32_t no_entry = 0xffffffff;
  std::array<std::uint8_t, 80> header{};
  if (size < header.size() || ::lseek(descriptor, static_cast<off_t>(offset), SEEK_SET) < 0) {
    throw std::runtime_error("The update has a truncated RomFS resource header.");
  }
  read_exact(descriptor, header.data(), header.size());
  if (little64(header.data()) != header.size()) throw std::runtime_error("The update has an invalid RomFS resource header.");
  std::array<std::pair<std::uint64_t, std::uint64_t>, 4> tables{};
  for (std::size_t index = 0; index < tables.size(); ++index) {
    const auto table_offset = little64(header.data() + 8 + index * 16);
    const auto table_size = little64(header.data() + 16 + index * 16);
    if (table_offset < header.size() || table_offset > size || table_size > size - table_offset ||
        table_offset % 4 || table_size % 4 || ((index == 0 || index == 2) && table_size < 4) ||
        (index == 1 && table_size < 24)) {
      throw std::runtime_error("The update has RomFS metadata outside its resource section.");
    }
    tables[index] = {table_offset, table_size};
    for (std::size_t previous = 0; previous < index; ++previous) {
      const auto [prior_offset, prior_size] = tables[previous];
      if (table_size && prior_size && table_offset < prior_offset + prior_size && prior_offset < table_offset + table_size) {
        throw std::runtime_error("The update has overlapping RomFS metadata tables.");
      }
    }
  }
  const auto data_offset = little64(header.data() + 72);
  if (data_offset < header.size() || data_offset >= size) throw std::runtime_error("The update has an invalid RomFS file-data offset.");
  for (const auto& [table_offset, table_size] : tables) {
    if (table_size && data_offset >= table_offset && data_offset < table_offset + table_size) {
      throw std::runtime_error("The update's RomFS file-data offset overlaps metadata.");
    }
  }
  const auto directory_offset = tables[1].first;
  const auto directory_size = tables[1].second;
  const auto [file_offset, file_size] = tables[3];
  std::array<std::uint8_t, 24> root{};
  if (::lseek(descriptor, static_cast<off_t>(offset + directory_offset), SEEK_SET) < 0) throw std::runtime_error("Cannot read the update's RomFS root.");
  read_exact(descriptor, root.data(), root.size());
  const auto directory_reference = [directory_size](std::uint32_t entry) {
    return entry == no_entry || (entry % 4 == 0 && entry <= directory_size - 24);
  };
  const auto child_directory = little32(root.data() + 8);
  const auto child_file = little32(root.data() + 12);
  if (little32(root.data()) != 0 || little32(root.data() + 4) != no_entry || little32(root.data() + 20) != 0 ||
      !directory_reference(child_directory) || child_directory == 0 || !directory_reference(little32(root.data() + 16)) ||
      (child_file != no_entry && (child_file % 4 || file_size < 32 || child_file > file_size - 32))) {
    throw std::runtime_error("The update has an invalid RomFS root directory.");
  }
  if (child_file != no_entry) {
    std::array<std::uint8_t, 32> file{};
    if (::lseek(descriptor, static_cast<off_t>(offset + file_offset + child_file), SEEK_SET) < 0) throw std::runtime_error("Cannot read the update's RomFS file metadata.");
    read_exact(descriptor, file.data(), file.size());
    const auto contents_offset = little64(file.data() + 8);
    const auto contents_size = little64(file.data() + 16);
    if (little32(file.data()) != 0 || little32(file.data() + 28) > file_size - child_file - file.size() ||
        contents_offset > size - data_offset || contents_size > size - data_offset - contents_offset) {
      throw std::runtime_error("The update has a RomFS root file outside its resource section.");
    }
  }
}

void validate_nro(int descriptor, std::uint64_t size, bool require_resources = false) {
  if (size < 0x80 || size > max_update_size || ::lseek(descriptor, 0, SEEK_SET) < 0) throw std::runtime_error("The update is not a supported NRO executable.");
  std::array<std::uint8_t, 256> header{};
  const auto header_size = static_cast<std::size_t>(std::min<std::uint64_t>(header.size(), size));
  read_exact(descriptor, header.data(), header_size);
  if (!wodex_nro_validate(header.data(), header_size, size)) throw std::runtime_error("The update has an invalid NRO executable header.");
  const std::uint64_t code_size = little32(header.data() + 0x18);
  if (size == code_size) {
    if (require_resources) throw std::runtime_error("The update is missing its packaged RomFS resources.");
    return;
  }
  std::array<std::uint8_t, 56> aset{};
  if (size - code_size < aset.size() || ::lseek(descriptor, static_cast<off_t>(code_size), SEEK_SET) < 0) {
    throw std::runtime_error("The update has a truncated NRO asset header.");
  }
  read_exact(descriptor, aset.data(), aset.size());
  if (std::memcmp(aset.data(), "ASET", 4) != 0 || little32(aset.data() + 4) != 0) throw std::runtime_error("The update has an invalid NRO asset header.");
  std::array<std::pair<std::uint64_t, std::uint64_t>, 3> ranges{};
  for (std::size_t index = 0; index < ranges.size(); ++index) {
    const auto offset = little64(aset.data() + 8 + index * 16);
    const auto length = little64(aset.data() + 16 + index * 16);
    if (length && (offset < aset.size() || offset > size - code_size || length > size - code_size - offset)) {
      throw std::runtime_error("The update contains assets outside its NRO file.");
    }
    ranges[index] = {offset, length};
    for (std::size_t previous = 0; previous < index; ++previous) {
      const auto [prior_offset, prior_length] = ranges[previous];
      if (length && prior_length && offset < prior_offset + prior_length && prior_offset < offset + length) {
        throw std::runtime_error("The update contains overlapping NRO assets.");
      }
    }
  }
  if (require_resources) {
    const auto [romfs_offset, romfs_size] = ranges[2];
    if (!romfs_size) throw std::runtime_error("The update is missing its packaged RomFS resources.");
    validate_romfs(descriptor, code_size + romfs_offset, romfs_size);
  }
}

std::uint64_t file_size(int descriptor) {
  struct stat status{};
  if (::fstat(descriptor, &status) != 0 || !S_ISREG(status.st_mode) || status.st_size < 0) throw std::runtime_error("The NRO path is not a regular file.");
  return static_cast<std::uint64_t>(status.st_size);
}

Hash new_hash() {
  Hash hash{wodex_sha256_new(), &wodex_sha256_free};
  if (!hash) throw std::runtime_error("Cannot initialize update checksum verification.");
  return hash;
}

std::string finish_hash(WodexSha256* hash) {
  RustString digest{wodex_sha256_finish(hash), &wodex_string_free};
  if (!digest) throw std::runtime_error("Cannot finish update checksum verification.");
  return digest.get();
}

void verify_stage(const PendingUpdate& pending) {
  File file{open_read(pending.staged_path)};
  const auto size = file_size(file.get());
  if (size != pending.size) throw std::runtime_error("The downloaded update size changed. Download it again.");
  auto hash = new_hash();
  std::array<std::uint8_t, 65536> buffer{};
  while (true) {
    const ssize_t count = ::read(file.get(), buffer.data(), buffer.size());
    if (count < 0 && errno == EINTR) continue;
    if (count < 0) throw std::runtime_error("Cannot verify the downloaded update.");
    if (count == 0) break;
    if (!wodex_sha256_update(hash.get(), buffer.data(), static_cast<std::size_t>(count))) throw std::runtime_error("Cannot verify the update checksum.");
  }
  if (finish_hash(hash.get()) != pending.sha256) throw std::runtime_error("The update checksum does not match the published release.");
  validate_nro(file.get(), size, true);
}

void flush_directory(const std::string& path) {
#ifndef __SWITCH__
  const auto slash = path.find_last_of('/');
  const std::string parent = slash == 0 ? "/" : path.substr(0, slash);
  int flags = O_RDONLY;
#ifdef O_DIRECTORY
  flags |= O_DIRECTORY;
#endif
  File directory{::open(parent.c_str(), flags)};
  if (directory.get() < 0 || ::fsync(directory.get()) != 0) throw std::runtime_error("Cannot flush the update directory to storage.");
#else
  static_cast<void>(path);
#endif
}

void validate_pending(const PendingUpdate& pending) {
  validate_target_path(pending.target_path);
  if (pending.staged_path != pending.target_path + ".update.part" || pending.backup_path != backup_path(pending.target_path) ||
      pending.size < 0x80 || pending.size > max_update_size || pending.sha256.size() != 64 ||
      !std::all_of(pending.sha256.begin(), pending.sha256.end(), [](char byte) { return (byte >= '0' && byte <= '9') || (byte >= 'a' && byte <= 'f'); })) {
    throw std::runtime_error("The pending update has invalid local paths or integrity metadata.");
  }
}

}  // namespace

struct Updater::Impl {
  Impl(std::string version, std::string target, std::string test_base)
      : current_version(std::move(version)), target_path(std::move(target)), test_api_base(validate_test_base(test_base)) {}
  std::string current_version;
  std::string target_path;
  std::string test_api_base;
  std::thread worker;
  std::atomic<bool> cancelled{false};
  std::atomic<bool> working{false};
  mutable std::mutex mutex;
  UpdateStatus state;
  std::optional<PendingUpdate> pending;
  Json release;

  void finish(UpdatePhase phase, const std::string& message) {
    std::lock_guard<std::mutex> lock{mutex};
    state.phase = phase;
    state.message = message;
    working.store(false);
  }

  void prepare(UpdatePhase phase, const std::string& message) {
    if (working.load()) throw std::runtime_error("An update operation is already running.");
    if (worker.joinable()) worker.join();
    cancelled.store(false);
    std::lock_guard<std::mutex> lock{mutex};
    state.phase = phase;
    state.message = message;
    state.downloaded = 0;
    working.store(true);
  }

  void inspect(const std::string& repository) noexcept {
    try {
      if (!valid_repository(repository)) throw std::runtime_error("Set the update repository to owner/repository before checking.");
      std::string manifest;
      fetch("https://api.github.com/repos/" + repository + "/releases/latest", test_api_base, cancelled,
        max_release_size, [&manifest](const char* bytes, std::size_t size) { manifest.append(bytes, size); });
      RustString inspected{wodex_update_inspect(manifest.c_str(), current_version.c_str()), &wodex_string_free};
      if (!inspected) throw std::runtime_error("Cannot inspect the update release.");
      const Json result = Json::parse(inspected.get());
      if (!result.value("ok", false)) throw std::runtime_error(result.value("error", "The release does not contain a verified Wodex NRO."));
      if (result.value("available", false)) {
        const auto asset_url = github_url(result.at("release").at("download_url").get<std::string>());
        const auto asset_path = ascii_lower(url_part(asset_url.get(), CURLUPART_PATH));
        const std::string prefix = "/" + ascii_lower(repository) + "/releases/download/";
        if (asset_path.compare(0, prefix.size(), prefix) != 0) {
          throw std::runtime_error("The release asset belongs to a different repository. Check the update source.");
        }
      }
      if (cancelled.load()) throw Cancelled{};
      {
        std::lock_guard<std::mutex> lock{mutex};
        release = result.value("release", Json{});
        state.version = release.is_object() ? release.value("version", "") : "";
        state.notes = release.is_object() ? release.value("notes", "") : "";
        state.total = release.is_object() ? release.value("size", std::uint64_t{0}) : 0;
      }
      finish(result.value("available", false) ? UpdatePhase::Available : UpdatePhase::Current,
        result.value("available", false) ? "A newer verified release is available." : "You are running the latest release.");
    } catch (const Cancelled&) { finish(UpdatePhase::Cancelled, "Update check cancelled."); }
    catch (const std::exception& error) { finish(UpdatePhase::Error, error.what()); }
    catch (...) { finish(UpdatePhase::Error, "Cannot inspect the update release."); }
  }

  void transfer(Json selected) noexcept {
    std::string staged;
    bool created = false;
    try {
      validate_target_path(target_path);
      PendingUpdate candidate{target_path, target_path + ".update.part", backup_path(target_path),
        selected.at("sha256").get<std::string>(), selected.at("size").get<std::uint64_t>()};
      validate_pending(candidate);
      struct stat installed{};
      if (!file_status(target_path, installed) || !S_ISREG(installed.st_mode)) throw std::runtime_error("The running NRO file is missing or is not a regular file.");
      staged = candidate.staged_path;
      struct stat stale{};
      if (file_status(staged, stale)) {
        if (!S_ISREG(stale.st_mode) || ::unlink(staged.c_str()) != 0) throw std::runtime_error("Cannot remove an incomplete update file.");
      }
      int flags = O_WRONLY | O_CREAT | O_EXCL;
#ifdef O_NOFOLLOW
      flags |= O_NOFOLLOW;
#endif
      File file{::open(staged.c_str(), flags, 0600)};
      if (file.get() < 0) throw std::runtime_error("Cannot create the update file beside the running NRO.");
      created = true;
      auto hash = new_hash();
      std::uint64_t received = 0;
      fetch(selected.at("download_url").get<std::string>(), test_api_base, cancelled, candidate.size,
        [&](const char* bytes, std::size_t size) {
          std::size_t written = 0;
          while (written < size) {
            const ssize_t count = ::write(file.get(), bytes + written, size - written);
            if (count < 0 && errno == EINTR) continue;
            if (count <= 0) throw std::runtime_error("Cannot write the update file. Check free space.");
            written += static_cast<std::size_t>(count);
          }
          if (!wodex_sha256_update(hash.get(), reinterpret_cast<const std::uint8_t*>(bytes), size)) throw std::runtime_error("Cannot verify the update checksum.");
          received += size;
          std::lock_guard<std::mutex> lock{mutex};
          state.downloaded = received;
        });
      if (received != candidate.size) throw std::runtime_error("The downloaded update size does not match the published release.");
      if (finish_hash(hash.get()) != candidate.sha256) throw std::runtime_error("The update checksum does not match the published release.");
      if (cancelled.load()) throw Cancelled{};
      file.finish();
      verify_stage(candidate);
      flush_directory(staged);
      if (cancelled.load()) throw Cancelled{};
      {
        std::lock_guard<std::mutex> lock{mutex};
        pending = candidate;
      }
      finish(UpdatePhase::Ready, "Choose Install and exit to apply the verified update, then launch Wodex again.");
      return;
    } catch (const Cancelled&) {
      if (created) ::unlink(staged.c_str());
      finish(UpdatePhase::Cancelled, "Update download cancelled.");
    } catch (const std::exception& error) {
      if (created) ::unlink(staged.c_str());
      finish(UpdatePhase::Error, error.what());
    } catch (...) {
      if (created) ::unlink(staged.c_str());
      finish(UpdatePhase::Error, "Cannot download the update.");
    }
  }
};

Updater::Updater(std::string current_version, std::string target_path, std::string test_api_base) {
  static CurlGlobal global;
  static_cast<void>(global);
  impl_ = std::make_unique<Impl>(std::move(current_version), std::move(target_path), std::move(test_api_base));
}

Updater::~Updater() {
  cancel();
  if (impl_->worker.joinable()) impl_->worker.join();
}

void Updater::check(std::string repository) {
  impl_->prepare(UpdatePhase::Checking, "Checking the configured GitHub repository…");
  {
    std::lock_guard<std::mutex> lock{impl_->mutex};
    if (impl_->pending) ::unlink(impl_->pending->staged_path.c_str());
    impl_->pending.reset();
    impl_->release = Json{};
    impl_->state.version.clear();
    impl_->state.notes.clear();
    impl_->state.total = 0;
  }
  try { impl_->worker = std::thread{&Impl::inspect, impl_.get(), std::move(repository)}; }
  catch (...) { impl_->finish(UpdatePhase::Error, "Cannot start the update worker."); }
}

void Updater::download() {
  Json selected;
  {
    std::lock_guard<std::mutex> lock{impl_->mutex};
    if (impl_->working.load()) throw std::runtime_error("An update operation is already running.");
    if (impl_->state.phase != UpdatePhase::Available || !impl_->release.is_object()) {
      impl_->state.phase = UpdatePhase::Error;
      impl_->state.message = "Check for an available update before downloading.";
      return;
    }
    selected = impl_->release;
  }
  impl_->prepare(UpdatePhase::Downloading, "Downloading and verifying the update…");
  try { impl_->worker = std::thread{&Impl::transfer, impl_.get(), std::move(selected)}; }
  catch (...) { impl_->finish(UpdatePhase::Error, "Cannot start the update worker."); }
}

void Updater::cancel() { impl_->cancelled.store(true); }
bool Updater::active() const { return impl_->working.load(); }
UpdateStatus Updater::status() const { std::lock_guard<std::mutex> lock{impl_->mutex}; return impl_->state; }
std::optional<PendingUpdate> Updater::pending_install() const {
  std::lock_guard<std::mutex> lock{impl_->mutex};
  return impl_->pending;
}

namespace detail {

// The operation seam enables deterministic rename/rollback fault tests without
// adding a runtime test switch or exposing installation controls in the UI.
void install_update_with_rename(const PendingUpdate& pending, int (*rename_file)(const char*, const char*)) {
  validate_pending(pending);
  struct stat current{}, staged{}, backup{};
  if (!file_status(pending.target_path, current) || !S_ISREG(current.st_mode) ||
      !file_status(pending.staged_path, staged) || !S_ISREG(staged.st_mode)) {
    throw std::runtime_error("The current or staged NRO is missing or is not a regular file. Nothing was replaced.");
  }
  verify_stage(pending);
  {
    File installed{open_read(pending.target_path)};
    validate_nro(installed.get(), file_size(installed.get()));
  }
  if (file_status(pending.backup_path, backup)) {
    if (!S_ISREG(backup.st_mode)) throw std::runtime_error("The previous-version backup path is not a regular file. Nothing was replaced.");
    if (::unlink(pending.backup_path.c_str()) != 0) throw std::runtime_error("Cannot rotate the previous-version backup. Nothing was replaced.");
  }
  if (rename_file(pending.target_path.c_str(), pending.backup_path.c_str()) != 0) {
    throw std::runtime_error("Cannot back up the running NRO. The installed version was not replaced.");
  }
  if (rename_file(pending.staged_path.c_str(), pending.target_path.c_str()) != 0) {
    if (rename_file(pending.backup_path.c_str(), pending.target_path.c_str()) != 0) {
      throw std::runtime_error("Installation failed. Your previous version is preserved in .previous.nro; launch it manually.");
    }
    throw std::runtime_error("Installation failed. The previous installed version was restored.");
  }
  flush_directory(pending.target_path);
}

}  // namespace detail

void install_update(const PendingUpdate& pending) { detail::install_update_with_rename(pending, &::rename); }

}  // namespace wodex
