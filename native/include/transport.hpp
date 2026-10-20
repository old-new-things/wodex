#pragma once

#include <atomic>
#include <cstddef>
#include <deque>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace wodex {

struct TransportEvent {
  enum Kind { Data, Finished };
  Kind kind = Data;
  std::string data;
  long http_status = 0;
  std::string error;
};

// Only the worker touches curl; only the UI touches application/Rust state.
class Transport {
public:
  Transport();
  ~Transport();
  Transport(const Transport&) = delete;
  Transport& operator=(const Transport&) = delete;

  // One request at a time; starting while active throws std::runtime_error.
  // Invalid input/setup failures produce a Finished event.
  void start(const std::string& url, const std::string& api_key, const std::string& body);
  void cancel();
  std::vector<TransportEvent> poll();
  bool active() const;

private:
  void run(std::string url, std::string api_key, std::string body) noexcept;
  void finish(long http_status, std::string error);
  bool receive(const char* data, std::size_t size);
  static std::size_t write_callback(char* data, std::size_t size, std::size_t count, void* context) noexcept;

  std::thread worker_;
  std::atomic<bool> cancelled_{false};
  std::atomic<bool> active_{false};
  std::mutex queue_mutex_;
  std::deque<TransportEvent> queue_;
  std::size_t queued_bytes_ = 0;
  bool buffer_full_ = false;
};

}  // namespace wodex
