#include "wodex_http.h"
#include <curl/curl.h>
#include <array>
#include <cstring>
#include <limits>
#include <memory>
#include <string>

namespace {
struct Easy {
  CURL* curl = curl_easy_init();
  curl_slist* headers = nullptr;
  void* context = nullptr;
  WodexHttpReceive body = nullptr;
  WodexHttpReceive header = nullptr;
  WodexHttpProgress progress = nullptr;
  ~Easy() {
    if (curl) curl_easy_cleanup(curl);
    for (auto* header = headers; header; header = header->next) {
      const size_t len = std::strlen(header->data);
      volatile char* text = header->data;
      for (size_t index = 0; index < len; ++index) text[index] = 0;
    }
    curl_slist_free_all(headers);
  }
};
struct CurlRuntime {
  bool ready = curl_global_init(CURL_GLOBAL_DEFAULT) == CURLE_OK;
  ~CurlRuntime() { if (ready) curl_global_cleanup(); }
};
bool init() noexcept {
  // Constructed before any worker; all Rust workers join before process exit.
  static const CurlRuntime runtime;
  return runtime.ready;
}
void* create() noexcept {
  try {
    auto easy = std::make_unique<Easy>();
    return easy->curl ? easy.release() : nullptr;
  } catch (...) { return nullptr; }
}
void destroy(void* handle) noexcept { delete static_cast<Easy*>(handle); }
bool set_string(void* handle, uint32_t option, const char* value) noexcept {
  auto* curl = static_cast<Easy*>(handle)->curl;
  switch (option) {
    case 1: return curl_easy_setopt(curl, CURLOPT_URL, value) == CURLE_OK;
    case 2: return curl_easy_setopt(curl, CURLOPT_CAINFO, value) == CURLE_OK;
    case 3: return curl_easy_setopt(curl, CURLOPT_USERAGENT, value) == CURLE_OK;
    case 4: return curl_easy_setopt(curl, CURLOPT_NOPROXY, value) == CURLE_OK;
#if LIBCURL_VERSION_NUM >= 0x075500
    case 5: return curl_easy_setopt(curl, CURLOPT_PROTOCOLS_STR, value) == CURLE_OK;
    case 6: return curl_easy_setopt(curl, CURLOPT_REDIR_PROTOCOLS_STR, value) == CURLE_OK;
#else
    case 5:
    case 6: {
      long protocols = 0;
      if (std::strcmp(value, "http") == 0) protocols = CURLPROTO_HTTP;
      else if (std::strcmp(value, "https") == 0) protocols = CURLPROTO_HTTPS;
      else if (std::strcmp(value, "http,https") == 0) protocols = CURLPROTO_HTTP | CURLPROTO_HTTPS;
      else return false;
      return (option == 5 ? curl_easy_setopt(curl, CURLOPT_PROTOCOLS, protocols) :
        curl_easy_setopt(curl, CURLOPT_REDIR_PROTOCOLS, protocols)) == CURLE_OK;
    }
#endif
    default: return false;
  }
}
bool set_number(void* handle, uint32_t option, int64_t value) noexcept {
  if (value < std::numeric_limits<long>::min() || value > std::numeric_limits<long>::max()) return false;
  auto* curl = static_cast<Easy*>(handle)->curl;
  const long number = static_cast<long>(value);
  switch (option) {
    case 1: return curl_easy_setopt(curl, CURLOPT_POST, number) == CURLE_OK;
    case 2: return curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, number) == CURLE_OK;
    case 3: return curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, number) == CURLE_OK;
    case 4: return curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, number) == CURLE_OK;
    case 5: return curl_easy_setopt(curl, CURLOPT_CONNECTTIMEOUT, number) == CURLE_OK;
    case 6: return curl_easy_setopt(curl, CURLOPT_TIMEOUT, number) == CURLE_OK;
    case 7: return curl_easy_setopt(curl, CURLOPT_LOW_SPEED_LIMIT, number) == CURLE_OK;
    case 8: return curl_easy_setopt(curl, CURLOPT_LOW_SPEED_TIME, number) == CURLE_OK;
    case 9: return curl_easy_setopt(curl, CURLOPT_NOSIGNAL, number) == CURLE_OK;
    case 10: return curl_easy_setopt(curl, CURLOPT_NOPROGRESS, number) == CURLE_OK;
    case 11: return curl_easy_setopt(curl, CURLOPT_TCP_KEEPALIVE, number) == CURLE_OK;
    default: return false;
  }
}
bool set_body(void* handle, const uint8_t* bytes, size_t len) noexcept {
  auto* curl = static_cast<Easy*>(handle)->curl;
  if (len > static_cast<uint64_t>(std::numeric_limits<curl_off_t>::max())) return false;
  return curl_easy_setopt(curl, CURLOPT_POSTFIELDS, bytes) == CURLE_OK &&
    curl_easy_setopt(curl, CURLOPT_POSTFIELDSIZE_LARGE, static_cast<curl_off_t>(len)) == CURLE_OK;
}
bool append_header(void* handle, const char* value) noexcept {
  auto* easy = static_cast<Easy*>(handle);
  auto* headers = curl_slist_append(easy->headers, value);
  if (!headers) return false;
  easy->headers = headers;
  return curl_easy_setopt(easy->curl, CURLOPT_HTTPHEADER, headers) == CURLE_OK;
}
size_t receive_body(char* bytes, size_t size, size_t count, void* context) noexcept {
  if (size && count > std::numeric_limits<size_t>::max() / size) return 0;
  auto* easy = static_cast<Easy*>(context);
  return easy->body(easy->context, reinterpret_cast<const uint8_t*>(bytes), size * count);
}
size_t receive_header(char* bytes, size_t size, size_t count, void* context) noexcept {
  if (size && count > std::numeric_limits<size_t>::max() / size) return 0;
  auto* easy = static_cast<Easy*>(context);
  return easy->header(easy->context, reinterpret_cast<const uint8_t*>(bytes), size * count);
}
int progress(void* context, curl_off_t, curl_off_t, curl_off_t, curl_off_t) noexcept {
  auto* easy = static_cast<Easy*>(context);
  return easy->progress(easy->context) ? 1 : 0;
}
bool callbacks(void* handle, void* context, WodexHttpReceive body, WodexHttpReceive header,
               WodexHttpProgress progress_callback) noexcept {
  auto* easy = static_cast<Easy*>(handle);
  easy->context = context;
  easy->body = body;
  easy->header = header;
  easy->progress = progress_callback;
  return curl_easy_setopt(easy->curl, CURLOPT_WRITEFUNCTION, receive_body) == CURLE_OK &&
    curl_easy_setopt(easy->curl, CURLOPT_WRITEDATA, easy) == CURLE_OK &&
    curl_easy_setopt(easy->curl, CURLOPT_HEADERFUNCTION, receive_header) == CURLE_OK &&
    curl_easy_setopt(easy->curl, CURLOPT_HEADERDATA, easy) == CURLE_OK &&
    curl_easy_setopt(easy->curl, CURLOPT_XFERINFOFUNCTION, progress) == CURLE_OK &&
    curl_easy_setopt(easy->curl, CURLOPT_XFERINFODATA, easy) == CURLE_OK;
}
uint32_t perform(void* handle) noexcept {
  return static_cast<uint32_t>(curl_easy_perform(static_cast<Easy*>(handle)->curl));
}
uint16_t status(void* handle) noexcept {
  long result = 0;
  curl_easy_getinfo(static_cast<Easy*>(handle)->curl, CURLINFO_RESPONSE_CODE, &result);
  return result >= 0 && result <= 65535 ? static_cast<uint16_t>(result) : 0;
}
struct Url {
  CURLU* handle = curl_url();
  std::array<std::string, 9> parts;
  std::array<bool, 9> present{};
  ~Url() { if (handle) curl_url_cleanup(handle); }
};
void* url_parse(const char* base, const char* value) noexcept {
  try {
    auto url = std::make_unique<Url>();
    if (!url->handle || (*base && curl_url_set(url->handle, CURLUPART_URL, base, 0) != CURLUE_OK) ||
        curl_url_set(url->handle, CURLUPART_URL, value, 0) != CURLUE_OK) return nullptr;
    const std::array<CURLUPart, 9> names = {CURLUPART_URL, CURLUPART_SCHEME, CURLUPART_HOST,
      CURLUPART_PORT, CURLUPART_PATH, CURLUPART_QUERY, CURLUPART_USER, CURLUPART_PASSWORD, CURLUPART_FRAGMENT};
    for (size_t index = 0; index < names.size(); ++index) {
      char* raw = nullptr;
      const auto result = curl_url_get(url->handle, names[index], &raw, 0);
      std::unique_ptr<char, decltype(&curl_free)> owned{raw, curl_free};
      url->present[index] = result == CURLUE_OK;
      if (result == CURLUE_OK && raw) url->parts[index] = raw;
    }
    return url.release();
  } catch (...) { return nullptr; }
}
WodexUrlPart url_part(void* handle, uint32_t part) noexcept {
  auto* url = static_cast<Url*>(handle);
  if (part >= url->parts.size()) return {nullptr, 0, false};
  return {reinterpret_cast<const uint8_t*>(url->parts[part].data()), url->parts[part].size(), url->present[part]};
}
void url_free(void* handle) noexcept { delete static_cast<Url*>(handle); }
}
extern "C" const WodexHttpApi* wodex_http_api(void) {
  static const WodexHttpApi api = {init, create, destroy, set_string, set_number, set_body,
    append_header, callbacks, perform, status, url_parse, url_part, url_free};
  return &api;
}
