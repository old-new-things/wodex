#pragma once
#include <memory>
#include <string>
#include <vector>
namespace wodex {
// Test-only facade: every operation delegates to the production Rust backend.
struct TransportEvent {
  enum Kind { Data, Finished };
  Kind kind = Data;
  std::string data;
  long http_status = 0;
  std::string error;
};
class Transport {
public:
  Transport();
  ~Transport();
  Transport(const Transport&) = delete;
  Transport& operator=(const Transport&) = delete;
  void start(const std::string& url, const std::string& key, const std::string& body);
  void cancel();
  std::vector<TransportEvent> poll();
  bool active() const;
private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};
}
