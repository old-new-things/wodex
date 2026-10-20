// Native harness compatibility only; production uses the Rust backends directly.
#include "transport.hpp"
#include "updater.hpp"
#include "wodex_backend.h"
#include "wodex_core.h"
#include <cstdlib>
#include <memory>
#include <nlohmann/json.hpp>
#include <stdexcept>
#include <utility>

namespace wodex {
namespace {
using Json = nlohmann::json;
WodexBackendApi api() {
  return {wodex_native_fs_api(), *wodex_runtime_api(), *wodex_http_api()};
}
std::string ca_bundle() {
  const char *value = std::getenv("WODEX_CA_BUNDLE");
  return value ? value : "";
}
void checked(bool success, char *error) {
  std::unique_ptr<char, decltype(&wodex_string_free)> owned(error, wodex_string_free);
  if (!success) throw std::runtime_error(error ? error : "The Rust backend ran out of memory.");
}
Json owned_json(char *raw) {
  std::unique_ptr<char, decltype(&wodex_string_free)> owned(raw, wodex_string_free);
  if (!raw) throw std::runtime_error("The Rust backend returned no data.");
  return Json::parse(raw);
}
Json pending_json(const PendingUpdate &pending) {
  return {{"target_path", pending.target_path}, {"staged_path", pending.staged_path},
    {"backup_path", pending.backup_path}, {"sha256", pending.sha256}, {"size", pending.size}};
}
}
struct Transport::Impl {
  std::unique_ptr<WodexTransport, decltype(&wodex_transport_free)> value{nullptr, wodex_transport_free};
};
Transport::Transport() : impl_(std::make_unique<Impl>()) {
  const auto bindings = api();
  char *error = nullptr;
  impl_->value.reset(wodex_transport_new(&bindings, ca_bundle().c_str(), &error));
  checked(impl_->value != nullptr, error);
}
Transport::~Transport() = default;
void Transport::start(const std::string &url, const std::string &key, const std::string &body) {
  if (url.find('\0') != std::string::npos || key.find('\0') != std::string::npos || body.find('\0') != std::string::npos)
    throw std::runtime_error("Cannot marshal NUL-containing transport arguments.");
  char *error = nullptr;
  const bool success = wodex_transport_start(impl_->value.get(), url.c_str(), key.c_str(), body.c_str(), &error);
  checked(success, error);
}
void Transport::cancel() { wodex_transport_cancel(impl_->value.get()); }
bool Transport::active() const { return wodex_transport_active(impl_->value.get()); }
std::vector<TransportEvent> Transport::poll() {
  std::vector<TransportEvent> events;
  while (true) {
    const auto event = wodex_transport_poll(impl_->value.get());
    if (!event.kind) break;
    events.push_back({event.kind == 1 ? TransportEvent::Data : TransportEvent::Finished,
      event.length ? std::string(reinterpret_cast<const char *>(event.data), event.length) : "",
      event.http_status, event.error ? event.error : ""});
  }
  return events;
}
struct Updater::Impl {
  std::unique_ptr<WodexUpdater, decltype(&wodex_updater_free)> value{nullptr, wodex_updater_free};
};
Updater::Updater(std::string version, std::string target, std::string test_base) : impl_(std::make_unique<Impl>()) {
  const auto bindings = api();
  char *error = nullptr;
  impl_->value.reset(wodex_updater_new(&bindings, version.c_str(), target.c_str(), test_base.c_str(), ca_bundle().c_str(), &error));
  checked(impl_->value != nullptr, error);
}
Updater::~Updater() = default;
void Updater::check(std::string repository, UpdateChannel channel) {
  char *error = nullptr;
  const bool success = channel == UpdateChannel::Version
    ? wodex_updater_check(impl_->value.get(), repository.c_str(), &error)
    : wodex_updater_check_channel(impl_->value.get(), repository.c_str(), "nightly", &error);
  checked(success, error);
}
void Updater::download() {
  char *error = nullptr;
  const bool success = wodex_updater_download(impl_->value.get(), &error);
  checked(success, error);
}
void Updater::cancel() { wodex_updater_cancel(impl_->value.get()); }
bool Updater::active() const { return wodex_updater_active(impl_->value.get()); }
UpdateStatus Updater::status() const {
  const auto result = owned_json(wodex_updater_status(impl_->value.get()));
  return {static_cast<UpdatePhase>(result.at("phase").get<int>()), result.at("message"),
    result.at("version"), result.at("notes"), result.at("downloaded"), result.at("total")};
}
std::optional<PendingUpdate> Updater::pending_install() const {
  const auto result = owned_json(wodex_updater_pending(impl_->value.get()));
  if (result.is_null()) return {};
  return PendingUpdate{result.at("target_path"), result.at("staged_path"),
    result.at("backup_path"), result.at("sha256"), result.at("size")};
}
namespace detail {
void install_update_with_rename(const PendingUpdate &pending, int (*rename_file)(const char *, const char *)) {
  const auto fs = wodex_native_fs_api();
  const auto data = pending_json(pending).dump();
  char *error = nullptr;
  const bool success = wodex_update_install_json(&fs, data.c_str(), rename_file, &error);
  checked(success, error);
}
}
void install_update(const PendingUpdate &pending) { detail::install_update_with_rename(pending, nullptr); }
}
