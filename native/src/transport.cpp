#include "transport.hpp"

#include <algorithm>
#include <cstdlib>
#include <limits>
#include <memory>
#include <stdexcept>
#include <utility>

#include <curl/curl.h>

namespace wodex {
namespace {

constexpr std::size_t max_queue_bytes = 8 * 1024 * 1024;
constexpr std::size_t chunk_size = 16 * 1024;
constexpr std::size_t max_request_bytes = 4 * 1024 * 1024;

struct CurlGlobal {
  CurlGlobal() {
    if (curl_global_init(CURL_GLOBAL_DEFAULT) != CURLE_OK) {
      throw std::runtime_error("Cannot initialize HTTPS transport.");
    }
  }
  ~CurlGlobal() { curl_global_cleanup(); }
};

using Easy = std::unique_ptr<CURL, decltype(&curl_easy_cleanup)>;
using Url = std::unique_ptr<CURLU, decltype(&curl_url_cleanup)>;
using CurlString = std::unique_ptr<char, decltype(&curl_free)>;
using Headers = std::unique_ptr<curl_slist, decltype(&curl_slist_free_all)>;

bool has_control(const std::string& text) {
  return std::any_of(text.begin(), text.end(), [](unsigned char value) {
    return value < 0x20 || value == 0x7f;
  });
}

std::string url_part(CURLU* url, CURLUPart part) {
  char* text = nullptr;
  const CURLUcode status = curl_url_get(url, part, &text, 0);
  CurlString owned{text, &curl_free};
  return status == CURLUE_OK && text != nullptr ? std::string{text} : std::string{};
}

bool validate_url(const std::string& text) {
  if (text.empty() || text.size() > 4096 || has_control(text)) return false;
  Url url{curl_url(), &curl_url_cleanup};
  if (!url || curl_url_set(url.get(), CURLUPART_URL, text.c_str(), 0) != CURLUE_OK) return false;
  const std::string scheme = url_part(url.get(), CURLUPART_SCHEME);
  const std::string host = url_part(url.get(), CURLUPART_HOST);
  if (host.empty() || !url_part(url.get(), CURLUPART_USER).empty() ||
      !url_part(url.get(), CURLUPART_PASSWORD).empty() ||
      !url_part(url.get(), CURLUPART_FRAGMENT).empty()) return false;
  if (scheme == "https") return true;
  // Explicit loopback URLs support the local mock server without weakening TLS.
  return scheme == "http" && (host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1");
}

void wipe(std::string& text) {
  volatile char* data = text.empty() ? nullptr : &text[0];
  for (std::size_t index = 0; index < text.size(); ++index) data[index] = 0;
  text.clear();
}

std::string transfer_error(CURLcode result) {
  switch (result) {
    case CURLE_COULDNT_RESOLVE_HOST: return "Cannot resolve the API server. Check the endpoint and network.";
    case CURLE_COULDNT_RESOLVE_PROXY: return "Cannot resolve the configured network proxy.";
    case CURLE_COULDNT_CONNECT: return "Cannot connect to the API server.";
    case CURLE_OPERATION_TIMEDOUT: return "The request timed out. Please try again.";
    case CURLE_PEER_FAILED_VERIFICATION: return "The API server's TLS certificate could not be verified.";
    case CURLE_SSL_CACERT_BADFILE: return "Cannot read the trusted TLS certificate bundle.";
    case CURLE_SSL_CONNECT_ERROR: return "A secure connection to the API server could not be established.";
    default: return "The network request failed. Check your connection and try again.";
  }
}

// HTTP error bodies are deliberately not surfaced: providers can echo credentials
// or prompts in them. Status-specific guidance is enough to recover safely.
std::string http_error(long status) {
  if (status == 401 || status == 403) return "API access denied. Check your API key and account permissions.";
  if (status == 402) return "The API account has insufficient balance.";
  if (status == 429) return "The API rate limit was reached. Wait a moment and try again.";
  if (status >= 300 && status < 400) return "The API returned a redirect. Enter its final HTTPS endpoint in Settings.";
  if (status >= 500) return "The API server is temporarily unavailable. Please try again.";
  return "The API rejected the request (HTTP " + std::to_string(status) + "). Check your model and endpoint.";
}

struct ResponseContext {
  Transport* transport;
  CURL* easy;
};

}  // namespace

Transport::Transport() {
  // Construction happens on the main thread before any curl worker is created.
  static CurlGlobal global;
  static_cast<void>(global);
}

Transport::~Transport() {
  cancel();
  if (worker_.joinable()) worker_.join();
}

void Transport::start(const std::string& url, const std::string& api_key, const std::string& body) {
  if (active()) throw std::runtime_error("A request is already in progress.");
  if (worker_.joinable()) worker_.join();
  {
    std::lock_guard<std::mutex> lock{queue_mutex_};
    queue_.clear();
    queued_bytes_ = 0;
  }
  cancelled_.store(false);
  buffer_full_ = false;
  active_.store(true);
  if (!validate_url(url)) {
    finish(0, "Use an HTTPS API endpoint. Plain HTTP is allowed only for a local mock server.");
    return;
  }
  if (api_key.empty() || api_key.size() > 4096 || has_control(api_key)) {
    finish(0, "Enter a valid API key without control characters.");
    return;
  }
  if (body.empty() || body.size() > max_request_bytes) {
    finish(0, "The request is empty or exceeds the 4 MiB limit.");
    return;
  }
  try {
    worker_ = std::thread{&Transport::run, this, url, api_key, body};
  } catch (...) {
    finish(0, "Cannot start the network worker.");
  }
}

void Transport::cancel() { cancelled_.store(true); }
bool Transport::active() const { return active_.load(); }

std::vector<TransportEvent> Transport::poll() {
  std::lock_guard<std::mutex> lock{queue_mutex_};
  std::vector<TransportEvent> events;
  events.reserve(queue_.size());
  while (!queue_.empty()) {
    events.push_back(std::move(queue_.front()));
    queue_.pop_front();
  }
  queued_bytes_ = 0;
  return events;
}

void Transport::finish(long http_status, std::string error) {
  {
    std::lock_guard<std::mutex> lock{queue_mutex_};
    queue_.push_back(TransportEvent{TransportEvent::Finished, {}, http_status, std::move(error)});
    // A consumer that sees Finished must also see an inactive transport.
    active_.store(false);
  }
}

bool Transport::receive(const char* data, std::size_t size) {
  if (cancelled_.load()) return false;
  std::lock_guard<std::mutex> lock{queue_mutex_};
  if (size > max_queue_bytes - queued_bytes_) {
    buffer_full_ = true;
    return false;
  }
  while (size > 0) {
    if (queue_.empty() || queue_.back().kind != TransportEvent::Data || queue_.back().data.size() == chunk_size) {
      queue_.push_back(TransportEvent{});
    }
    auto& chunk = queue_.back().data;
    const std::size_t count = std::min(size, chunk_size - chunk.size());
    chunk.append(data, count);
    queued_bytes_ += count;
    data += count;
    size -= count;
  }
  return true;
}

std::size_t Transport::write_callback(char* data, std::size_t size, std::size_t count, void* context) noexcept {
  if (size != 0 && count > std::numeric_limits<std::size_t>::max() / size) return 0;
  const std::size_t bytes = size * count;
  auto* response = static_cast<ResponseContext*>(context);
  try {
    long status = 0;
    curl_easy_getinfo(response->easy, CURLINFO_RESPONSE_CODE, &status);
    if (status < 200 || status >= 300) return bytes;
    return response->transport->receive(data, bytes) ? bytes : 0;
  } catch (...) {
    return 0;
  }
}

void Transport::run(std::string url, std::string api_key, std::string body) noexcept {
  long http_status = 0;
  try {
    Easy easy{curl_easy_init(), &curl_easy_cleanup};
    if (!easy) throw std::runtime_error("Cannot create the HTTPS request.");
    std::string authorization = "Authorization: Bearer " + api_key;
    curl_slist* raw_headers = nullptr;
    const auto add_header = [&raw_headers](const char* header) {
      curl_slist* updated = curl_slist_append(raw_headers, header);
      if (updated == nullptr) throw std::bad_alloc{};
      raw_headers = updated;
    };
    Headers headers{nullptr, &curl_slist_free_all};
    try {
      add_header("Content-Type: application/json");
      add_header("Accept: text/event-stream");
      add_header("Expect:");
      add_header(authorization.c_str());
      headers.reset(raw_headers);
    } catch (...) {
      curl_slist_free_all(raw_headers);
      wipe(authorization);
      throw;
    }
    wipe(authorization);
    wipe(api_key);
    ResponseContext context{this, easy.get()};
    const auto option = [&easy](CURLoption name, auto value) {
      if (curl_easy_setopt(easy.get(), name, value) != CURLE_OK) {
        throw std::runtime_error("Cannot configure the secure network request.");
      }
    };
    option(CURLOPT_URL, url.c_str());
    if (url.compare(0, 7, "http://") == 0) option(CURLOPT_NOPROXY, "*");
    option(CURLOPT_POST, 1L);
    option(CURLOPT_HTTPHEADER, headers.get());
    option(CURLOPT_POSTFIELDS, body.data());
    option(CURLOPT_POSTFIELDSIZE_LARGE, static_cast<curl_off_t>(body.size()));
    option(CURLOPT_FOLLOWLOCATION, 0L);
#if LIBCURL_VERSION_NUM >= 0x075500
    option(CURLOPT_PROTOCOLS_STR, "http,https");
    option(CURLOPT_REDIR_PROTOCOLS_STR, "https");
#else
    option(CURLOPT_PROTOCOLS, static_cast<long>(CURLPROTO_HTTP | CURLPROTO_HTTPS));
    option(CURLOPT_REDIR_PROTOCOLS, static_cast<long>(CURLPROTO_HTTPS));
#endif
    option(CURLOPT_SSL_VERIFYPEER, 1L);
    option(CURLOPT_SSL_VERIFYHOST, 2L);
    option(CURLOPT_CONNECTTIMEOUT, 15L);
    option(CURLOPT_TIMEOUT, 600L);
    option(CURLOPT_LOW_SPEED_LIMIT, 1L);
    option(CURLOPT_LOW_SPEED_TIME, 90L);
    option(CURLOPT_NOSIGNAL, 1L);
    option(CURLOPT_NOPROGRESS, 0L);
    option(CURLOPT_TCP_KEEPALIVE, 1L);
    option(CURLOPT_WRITEFUNCTION, &Transport::write_callback);
    option(CURLOPT_WRITEDATA, &context);
    // curl_off_t is platform-defined; the lambda has its exact callback signature.
    option(CURLOPT_XFERINFOFUNCTION, +[](void* owner, curl_off_t, curl_off_t, curl_off_t, curl_off_t) -> int {
      return static_cast<Transport*>(owner)->cancelled_.load() ? 1 : 0;
    });
    option(CURLOPT_XFERINFODATA, this);
    option(CURLOPT_USERAGENT, "Wodex/0.1");
#ifdef __SWITCH__
    option(CURLOPT_CAINFO, "romfs:/cacert.pem");
#else
    const char* ca_bundle = std::getenv("WODEX_CA_BUNDLE");
    if (ca_bundle != nullptr && ca_bundle[0] != '\0') option(CURLOPT_CAINFO, ca_bundle);
#endif
    const CURLcode result = cancelled_.load() ? CURLE_ABORTED_BY_CALLBACK : curl_easy_perform(easy.get());
    curl_easy_getinfo(easy.get(), CURLINFO_RESPONSE_CODE, &http_status);
    wipe(body);
    if (cancelled_.load()) finish(http_status, "Request cancelled.");
    else if (buffer_full_) finish(http_status, "The response exceeded the 8 MiB buffer limit.");
    else if (result != CURLE_OK) finish(http_status, transfer_error(result));
    else if (http_status < 200 || http_status >= 300) finish(http_status, http_error(http_status));
    else finish(http_status, {});
  } catch (...) {
    wipe(api_key);
    wipe(body);
    finish(http_status, "Cannot complete the network request.");
  }
}

}  // namespace wodex
