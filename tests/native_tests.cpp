#include "platform.hpp"
#include "storage.hpp"
#include "transport.hpp"
#include "wodex_core.h"

#include <nlohmann/json.hpp>

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <sys/stat.h>
#include <thread>
#include <unistd.h>

namespace {

using Json = nlohmann::json;
using Clock = std::chrono::steady_clock;
constexpr const char* mock_key = "mock-local-test-key";
int passed = 0;

void check(bool condition, const char* description) {
  if (!condition) throw std::runtime_error(description);
  ++passed;
}

Json read_json(char* raw) {
  std::unique_ptr<char, decltype(&wodex_string_free)> owned{raw, &wodex_string_free};
  if (!raw) throw std::runtime_error("The Rust core returned a null response.");
  return Json::parse(raw);
}

class Core {
public:
  explicit Core(const char* saved = nullptr) : value_(wodex_new(saved), &wodex_free) {
    if (!value_) throw std::runtime_error("Cannot create the Rust core.");
  }

  Json call(const Json& command) { return read_json(wodex_call(value_.get(), command.dump().c_str())); }
  Json state() { return call({{"op", "snapshot"}}).at("state"); }
  Json feed(const std::string& data, bool fragment = false) {
    Json result;
    const std::size_t stride = fragment ? 1 : data.size();
    for (std::size_t offset = 0; offset < data.size(); offset += stride) {
      const auto count = std::min(stride, data.size() - offset);
      result = read_json(wodex_feed(value_.get(),
        reinterpret_cast<const std::uint8_t*>(data.data() + offset), count));
      if (!result.value("ok", false)) break;
    }
    return result;
  }

private:
  std::unique_ptr<WodexCore, decltype(&wodex_free)> value_;
};

Json request(Core& core, const std::string& endpoint) {
  const auto settings = core.call({{"op", "settings"}, {"settings", {
    {"api_key", mock_key}, {"base_url", endpoint}, {"model", "mock-model"}, {"thinking", true}
  }}});
  check(settings.value("ok", false), "The core rejected valid mock settings.");
  const auto response = core.call({{"op", "send"}, {"content", "Hello, Switch!"}});
  check(response.value("ok", false), "The core did not create a chat request.");
  return response.at("request");
}

void start(wodex::Transport& transport, const Json& request) {
  transport.start_request(request.dump());
}

struct Result {
  std::string data;
  std::string error;
  long status = 0;
};

Result drain(wodex::Transport& transport, Core* core = nullptr, bool fragment = false) {
  const auto deadline = Clock::now() + std::chrono::seconds{10};
  Result result;
  while (Clock::now() < deadline) {
    for (auto& event : transport.poll()) {
      if (event.kind == wodex::TransportEvent::Data) {
        result.data += event.data;
        if (core && core->state().value("is_streaming", false)) core->feed(event.data, fragment);
      } else {
        result.status = event.http_status;
        result.error = event.error;
        if (core) core->call({{"op", "finish"}, {"http_status", event.http_status}, {"error", event.error}});
        check(!transport.active(), "Finished was published before the transport became inactive.");
        return result;
      }
    }
    std::this_thread::sleep_for(std::chrono::milliseconds{2});
  }
  transport.cancel();
  throw std::runtime_error("The local mock request did not finish before its deadline.");
}

void test_normal(const std::string& base_url) {
  Core core;
  wodex::Transport transport;
  const auto outgoing = request(core, base_url + "/normal");
  const auto& body = outgoing.at("body");
  check(body.at("stream") == true && body.at("stream_options").at("include_usage") == true,
    "The request omitted streaming or usage options.");
  start(transport, outgoing);
  const auto result = drain(transport, &core, true);
  check(result.status == 200 && result.error.empty(), "The normal mock response failed.");
  const auto state = core.state();
  const auto& answer = state.at("conversations").at(0).at("messages").at(1);
  check(answer.at("content") == "你好，Switch! 🌿", "UTF-8 answer fragments were not preserved.");
  check(answer.at("reasoning_content") == "思考中…", "Reasoning fragments were not preserved.");
  check(answer.at("status") == "complete" && answer.at("finish_reason") == "stop",
    "The final answer state was incorrect.");
  check(answer.at("usage").at("total_tokens") == 15, "Usage was not retained.");
  check(state.at("is_streaming") == false && state.at("error").is_null(), "The stream did not end cleanly.");

  const auto retry = core.call({{"op", "retry"}});
  check(retry.at("ok") == true, "Retry did not prepare another request.");
  for (const auto& message : retry.at("request").at("body").at("messages")) {
    check(!message.contains("reasoning_content"), "Private reasoning was sent in request history.");
  }
  core.call({{"op", "cancel"}});
}

void test_http_errors(const std::string& base_url) {
  for (const auto& scenario : {std::pair{"unauthorized", 401L}, std::pair{"redirect", 302L}}) {
    Core core;
    wodex::Transport transport;
    start(transport, request(core, base_url + "/" + scenario.first));
    const auto result = drain(transport, &core);
    check(result.status == scenario.second && !result.error.empty(), "An HTTP error was not reported.");
    check(result.data.empty(), "An HTTP error body reached the chat parser.");
    check(result.error.find(mock_key) == std::string::npos &&
      result.error.find("private-error-body") == std::string::npos, "An HTTP error leaked private provider text.");
    const auto state = core.state();
    check(state.at("conversations").at(0).at("messages").at(1).at("status") == "error",
      "An HTTP failure was not retained as an error message.");
  }
}

void test_compatible_provider(const std::string& base_url) {
  for (const auto* token_field : {"max_tokens", "max_completion_tokens"}) {
    Core core;
    wodex::Transport transport;
    const auto settings = core.call({{"op", "settings"}, {"settings", {
      {"provider", "openai_compatible"}, {"api_key", "generic-local-test-key"},
      {"base_url", base_url + "/compatible/" + token_field}, {"model", "mock-model"},
      {"token_limit_field", token_field}
    }}});
    check(settings.at("ok") == true, "Compatible provider settings were rejected.");
    const auto response = core.call({{"op", "send"}, {"content", "Hello, Switch!"}});
    check(response.at("ok") == true, "Compatible provider could not create a request.");
    const auto& outgoing = response.at("request");
    check(outgoing.at("body").at(token_field) == 4096, "The token limit field was not respected.");
    for (const auto* field : {"thinking", "reasoning_effort", "stream_options"}) {
      check(!outgoing.at("body").contains(field), "A DeepSeek option reached the compatible provider.");
    }
    start(transport, outgoing);
    const auto result = drain(transport, &core, true);
    check(result.status == 200 && result.error.empty(), "Compatible provider HTTP streaming failed.");
    const auto state = core.state();
    const auto& answer = state.at("conversations").at(0).at("messages").at(1);
    check(answer.at("content") == "你好，Switch! 🌿" && answer.at("status") == "complete",
      "Compatible provider streaming text was not retained.");
    const auto saved = core.call({{"op", "export"}}).at("data").dump();
    Core restored{saved.c_str()};
    check(restored.state().at("settings").at("provider") == "openai_compatible",
      "The provider did not survive save/reload.");
  }
}

void test_named_providers_and_native_protocols(const std::string& base_url) {
  struct Scenario {
    const char* provider;
    const char* model;
    const char* protocol;
    const char* suffix;
    const char* token_field;
  };
  for (const auto& scenario : {
      Scenario{"openai", "mock-model", "chat_completions", "chat/completions", "max_completion_tokens"},
      Scenario{"opencode_go", "glm-4.7", "chat_completions", "chat/completions", "max_tokens"},
      Scenario{"llama_cpp", "mock-model", "chat_completions", "chat/completions", "max_tokens"},
      Scenario{"zai", "glm-4.7", "chat_completions", "chat/completions", "max_tokens"},
      Scenario{"zai_coding_plan", "glm-4.7", "chat_completions", "chat/completions", "max_tokens"},
      Scenario{"zhipuai", "glm-4.7", "chat_completions", "chat/completions", "max_tokens"},
      Scenario{"zhipuai_coding_plan", "glm-4.7", "chat_completions", "chat/completions", "max_tokens"},
      Scenario{"opencode_go", "gpt-4o", "responses", "responses", "max_output_tokens"},
      Scenario{"opencode_go", "minimax-m2.5", "messages", "messages", "max_tokens"},
    }) {
    Core core;
    wodex::Transport transport;
    const std::string key = std::string(scenario.provider) == "llama_cpp" ? "" :
      std::string{"provider-test-key-"} + scenario.provider;
    const std::string endpoint = base_url + "/providers/" + scenario.provider + "/" + scenario.protocol;
    const auto settings = core.call({{"op", "settings"}, {"settings", {
      {"provider", scenario.provider}, {"api_key", key}, {"base_url", endpoint},
      {"model", scenario.model}, {"max_tokens", 37},
      {"system_prompt", "Respond clearly. 包含中文。"}
    }}});
    check(settings.value("ok", false), "A named provider rejected valid loopback settings.");
    const auto response = core.call({{"op", "send"}, {"content", "Hello, Switch!"}});
    check(response.value("ok", false), "A named provider failed to prepare a request.");
    const auto& outgoing = response.at("request");
    check(outgoing.at("url") == endpoint + "/" + scenario.suffix &&
      outgoing.at("protocol") == scenario.protocol, "Tokn selected the wrong provider operation.");
    check(outgoing.at("body").at(scenario.token_field) == 37,
      "Tokn did not preserve the configured response token limit.");
    start(transport, outgoing);
    const auto result = drain(transport, &core, true);
    check(result.status == 200 && result.error.empty(), "A named provider failed HTTP streaming.");
    const auto state = core.state();
    const auto& answer = state.at("conversations").at(0).at("messages").at(1);
    check(answer.at("content") == "你好，Switch! 🌿" && answer.at("reasoning_content") == "思考中…",
      "Provider protocol conversion lost UTF-8 text or reasoning.");
    check(answer.at("status") == "complete" && answer.at("finish_reason") == "stop",
      "A native provider protocol did not finish the answer cleanly.");
    check(answer.at("usage").at("prompt_tokens") == 5 &&
      answer.at("usage").at("completion_tokens") == 10 && answer.at("usage").at("total_tokens") == 15,
      "A provider's usage counters were not normalized.");
    const auto saved = core.call({{"op", "export"}}).at("data").dump();
    Core restored{saved.c_str()};
    const auto restored_state = restored.state();
    check(restored_state.at("settings").at("provider") == scenario.provider &&
      restored_state.at("settings").at("api_key") == key,
      "A named provider's configuration did not survive save/reload.");
    check(restored_state.at("conversations").at(0).at("messages").at(1) == answer,
      "The normalized provider response did not survive save/reload.");
  }
}

void test_truncated(const std::string& base_url) {
  Core core;
  wodex::Transport transport;
  start(transport, request(core, base_url + "/truncated"));
  const auto result = drain(transport, &core);
  const auto state = core.state();
  const auto& answer = state.at("conversations").at(0).at("messages").at(1);
  check(result.status == 200 && result.error.empty(), "The truncated mock request did not reach the parser.");
  check(answer.at("content") == "Partial answer" && answer.at("status") == "error",
    "An unfinished SSE stream was treated as a complete answer.");
  check(state.at("error").is_string(), "A truncated stream did not produce a recoverable error.");
}

void test_cancel(const std::string& base_url) {
  Core core;
  wodex::Transport transport;
  start(transport, request(core, base_url + "/cancel"));
  const auto deadline = Clock::now() + std::chrono::seconds{5};
  bool received = false;
  while (!received && Clock::now() < deadline) {
    for (auto& event : transport.poll()) {
      if (event.kind == wodex::TransportEvent::Data) {
        core.feed(event.data);
        received = true;
      } else throw std::runtime_error("The cancellable response finished unexpectedly.");
    }
    if (!received) std::this_thread::sleep_for(std::chrono::milliseconds{2});
  }
  check(received, "The cancellable mock never started streaming.");
  const auto cancelled_at = Clock::now();
  core.call({{"op", "cancel"}});
  transport.cancel();
  check(Clock::now() - cancelled_at < std::chrono::milliseconds{100}, "Cancel blocked the UI thread.");
  const auto result = drain(transport, &core);
  check(Clock::now() - cancelled_at < std::chrono::seconds{3}, "The transport did not stop promptly after cancellation.");
  check(!result.error.empty(), "Cancellation did not produce a terminal transport event.");
  check(core.state().at("conversations").at(0).at("messages").at(1).at("status") == "interrupted",
    "Late transport completion overwrote the cancelled message status.");
}

void test_validation(const std::string& base_url) {
  const std::string good_url = base_url + "/normal/chat/completions";
  for (const auto& invalid : {std::string{"http://example.com/chat/completions"},
       std::string{"file:///etc/passwd"}, std::string{"https://user:password@example.com"},
       std::string{"https://api.deepseek.com/\nchat/completions"}}) {
    wodex::Transport transport;
    transport.start(invalid, mock_key, "{}");
    const auto result = drain(transport);
    check(result.status == 0 && !result.error.empty() && result.data.empty(), "An unsafe URL was accepted.");
  }
  wodex::Transport transport;
  transport.start(good_url, "mock-key\r\nInjected: value", "{}");
  const auto result = drain(transport);
  check(result.status == 0 && !result.error.empty(), "An injected API header was accepted.");
}

void test_prepared_request_validation(const std::string& base_url) {
  Core core;
  const auto outgoing = request(core, base_url + "/normal");
  for (const auto& headers : {
      Json::array({Json::array({"Authorization", "Bearer mock-key\r\nInjected: value"})}),
      Json::array({Json::array({"Host", "unrelated.example"})}),
      Json::array({Json::array({"Authorization", "Bearer key"}), Json::array({"authorization", "Bearer other"})}),
    }) {
    auto invalid = outgoing;
    invalid["headers"] = headers;
    wodex::Transport transport;
    start(transport, invalid);
    const auto result = drain(transport);
    check(result.status == 0 && !result.error.empty() && result.data.empty(),
      "An injected, disallowed or duplicate prepared header reached HTTP.");
  }
  auto invalid = outgoing;
  invalid["url"] = "http://example.com/v1/chat/completions";
  invalid["allow_insecure_http"] = true;
  wodex::Transport transport;
  start(transport, invalid);
  const auto result = drain(transport);
  check(result.status == 0 && !result.error.empty() && result.data.empty(),
    "Local HTTP opt-in allowed a public cleartext provider request.");
  invalid = outgoing;
  invalid["allow_insecure_http"] = "true";
  bool rejected = false;
  try { start(transport, invalid); } catch (const std::runtime_error&) { rejected = true; }
  check(rejected && !transport.active(), "A malformed local HTTP opt-in started a request.");
}

void test_bounded_queue(const std::string& base_url) {
  Core core;
  wodex::Transport transport;
  start(transport, request(core, base_url + "/oversized"));
  const auto deadline = Clock::now() + std::chrono::seconds{5};
  while (transport.active() && Clock::now() < deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds{5});
  }
  const auto result = drain(transport);
  check(result.data.size() <= 8 * 1024 * 1024, "The network queue exceeded its memory limit.");
  check(result.error.find("8 MiB") != std::string::npos, "An oversized response did not stop at the buffer limit.");
  core.call({{"op", "cancel"}});
}

void test_persistence() {
  char temporary[] = "/tmp/wodex-native-test-XXXXXX";
  char* directory = ::mkdtemp(temporary);
  if (!directory) throw std::runtime_error("Cannot create temporary storage for tests.");
  const std::filesystem::path root{directory};
  struct Cleanup {
    std::filesystem::path root;
    ~Cleanup() { std::error_code error; std::filesystem::remove_all(root, error); }
  } cleanup{root};
  const std::string path = (root / "state.json").string();
  Core core;
  const std::string original = core.call({{"op", "export"}}).at("data").dump();
  wodex::atomic_write(path, original);
  check(wodex::load_file(path) == original, "Saved state did not round-trip through durable storage.");
  struct stat status{};
  check(::stat(path.c_str(), &status) == 0 && (status.st_mode & 0777) == 0600,
    "Saved state permissions allow access by another user.");
  core.call({{"op", "new_chat"}});
  const std::string replacement = core.call({{"op", "export"}}).at("data").dump();
  wodex::atomic_write(path, replacement);
  Core restored{wodex::load_file(path).c_str()};
  check(restored.state().at("conversations").size() == 2, "Replacing saved state lost the new conversation.");
  check(wodex::load_file((root / "missing.json").string()).empty(), "A missing save was not handled as empty.");
  bool failed = false;
  try { wodex::atomic_write(path, std::string(32 * 1024 * 1024 + 1, 'x')); }
  catch (const std::runtime_error&) { failed = true; }
  check(failed && wodex::load_file(path) == replacement, "An invalid write damaged the previous save.");
  const std::string corrupt = "{a broken save that must be preserved";
  wodex::atomic_write(path, corrupt);
  Core broken{wodex::load_file(path).c_str()};
  check(broken.state().at("error").is_string(), "Corrupt saved data did not produce a recovery error.");
  check(broken.call({{"op", "export"}}).at("ok") == false,
    "A corrupt save can be overwritten before explicit recovery.");
  check(wodex::load_file(path) == corrupt, "Corrupt saved data was modified during loading.");
  const std::string earlier_backup = path + ".recovery-1";
  wodex::atomic_write(earlier_backup, "an earlier recovery that must survive");
  const std::string backup = wodex::preserve_file(path, corrupt);
  check(backup == path + ".recovery-2", "Recovery did not skip an existing backup name.");
  check(wodex::load_file(earlier_backup) == "an earlier recovery that must survive",
    "A recovery file from an earlier launch was overwritten.");
  const std::string later_backup = wodex::preserve_file(path, "another preserved version");
  check(later_backup == path + ".recovery-3" && wodex::load_file(backup) == corrupt,
    "Repeated recovery did not preserve all prior versions.");
  check(::stat(backup.c_str(), &status) == 0 && (status.st_mode & 0777) == 0600,
    "A recovery file allows access by another user.");
  check(wodex::load_file(path) == corrupt, "Creating a recovery file changed the source.");
  broken.call({{"op", "reset_storage"}});
  const auto recovered = broken.call({{"op", "export"}});
  check(recovered.at("ok") == true, "Explicit storage reset did not unblock export.");
  wodex::atomic_write(path, recovered.at("data").dump());
  check(wodex::load_file(backup) == corrupt,
    "Explicit recovery did not preserve the corrupt backup.");
}

}  // namespace

int main(int argc, char** argv) {
  try {
    if (argc != 2) throw std::runtime_error("Run this binary through tests/mock_deepseek.py.");
    const std::string base_url = argv[1];
    test_normal(base_url);
    test_compatible_provider(base_url);
    test_named_providers_and_native_protocols(base_url);
    test_http_errors(base_url);
    test_truncated(base_url);
    test_cancel(base_url);
    test_validation(base_url);
    test_prepared_request_validation(base_url);
    test_bounded_queue(base_url);
    test_persistence();
    std::cout << "Native integration: " << passed << " checks passed.\n";
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "Native integration failed: " << error.what() << '\n';
    return 1;
  }
}
