#pragma once

// Test-only facade around the production Rust updater and installer.

#include <cstdint>
#include <memory>
#include <optional>
#include <string>

namespace wodex {

enum class UpdatePhase { Idle, Checking, Current, Available, Downloading, Ready, Cancelled, Error };

struct UpdateStatus {
  UpdatePhase phase = UpdatePhase::Idle;
  std::string message;
  std::string version;
  std::string notes;
  std::uint64_t downloaded = 0;
  std::uint64_t total = 0;
};

struct PendingUpdate {
  std::string target_path;
  std::string staged_path;
  std::string backup_path;
  std::string sha256;
  std::uint64_t size = 0;
};

class Updater {
public:
  // An empty target enables release checking without executable replacement.
  // Tests may supply an explicit HTTP loopback origin. Canonical HTTPS GitHub
  // URLs then map to origin/<host>/<path>; production leaves this argument empty.
  Updater(std::string current_version, std::string target_path, std::string test_api_base = "");
  ~Updater();
  Updater(const Updater&) = delete;
  Updater& operator=(const Updater&) = delete;

  void check(std::string repository);
  void download();
  void cancel();
  bool active() const;
  UpdateStatus status() const;
  std::optional<PendingUpdate> pending_install() const;

private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

// Call only after the application, worker, fonts, and Platform RomFS mount have
// been destroyed. The installed file is moved to a visible .previous.nro backup.
// Power loss between the two renames may require launching that backup manually.
void install_update(const PendingUpdate& pending);

}  // namespace wodex
