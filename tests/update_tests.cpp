#include "updater.hpp"
#include "wodex_core.h"

#include <algorithm>
#include <array>
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <sys/stat.h>
#include <thread>
#include <unistd.h>
#include <vector>

namespace wodex::detail {
void install_update_with_rename(const PendingUpdate&, int (*rename_file)(const char*, const char*));
}

namespace {

using Clock = std::chrono::steady_clock;
using wodex::PendingUpdate;
using wodex::Updater;
using wodex::UpdatePhase;
int passed = 0;
int rename_count = 0;
bool fail_rollback = false;

void check(bool condition, const char* description) {
  if (!condition) throw std::runtime_error(description);
  ++passed;
}

template <typename Action> void rejects(Action action, const char* description) {
  bool rejected = false;
  try { action(); } catch (const std::exception&) { rejected = true; }
  check(rejected, description);
}

void little32(std::vector<std::uint8_t>& bytes, std::size_t offset, std::uint32_t value) {
  for (std::size_t index = 0; index < 4; ++index) bytes[offset + index] = static_cast<std::uint8_t>(value >> (index * 8));
}

void little64(std::vector<std::uint8_t>& bytes, std::size_t offset, std::uint64_t value) {
  for (std::size_t index = 0; index < 8; ++index) bytes[offset + index] = static_cast<std::uint8_t>(value >> (index * 8));
}

std::vector<std::uint8_t> nro(std::uint8_t marker = 1, std::size_t size = 4096) {
  std::vector<std::uint8_t> bytes(size, 0);
  std::copy_n("NRO0", 4, bytes.begin() + 0x10);
  little32(bytes, 0x18, 0x200);
  little32(bytes, 0x20, 0);
  little32(bytes, 0x24, 0x100);
  little32(bytes, 0x28, 0x100);
  little32(bytes, 0x2c, 0x80);
  little32(bytes, 0x30, 0x180);
  little32(bytes, 0x34, 0x80);
  bytes[0x90] = marker;
  std::copy_n("ASET", 4, bytes.begin() + 0x200);
  little64(bytes, 0x228, 56);
  little64(bytes, 0x230, size - 0x200 - 56);
  const std::size_t romfs = 0x200 + 56;
  const std::array<std::uint64_t, 10> header = {80, 80, 4, 84, 24, 108, 4, 112, 36, 160};
  for (std::size_t index = 0; index < header.size(); ++index) little64(bytes, romfs + index * 8, header[index]);
  little32(bytes, romfs + 84 + 4, 0xffffffff);
  little32(bytes, romfs + 84 + 8, 0xffffffff);
  little32(bytes, romfs + 84 + 16, 0xffffffff);
  little32(bytes, romfs + 112 + 4, 0xffffffff);
  little64(bytes, romfs + 112 + 16, 1);
  little32(bytes, romfs + 112 + 24, 0xffffffff);
  little32(bytes, romfs + 112 + 28, 4);
  std::copy_n("data", 4, bytes.begin() + static_cast<std::ptrdiff_t>(romfs + 144));
  bytes[romfs + 160] = 0x5a;
  return bytes;
}

std::string digest(const std::vector<std::uint8_t>& bytes) {
  std::unique_ptr<WodexSha256, decltype(&wodex_sha256_free)> hash{wodex_sha256_new(), &wodex_sha256_free};
  if (!hash) throw std::runtime_error("Cannot hash the update fixture.");
  for (std::size_t offset = 0; offset < bytes.size(); offset += 65536) {
    if (!wodex_sha256_update(hash.get(), bytes.data() + offset, std::min<std::size_t>(65536, bytes.size() - offset))) {
      throw std::runtime_error("Cannot hash the update fixture.");
    }
  }
  std::unique_ptr<char, decltype(&wodex_string_free)> value{wodex_sha256_finish(hash.get()), &wodex_string_free};
  if (!value) throw std::runtime_error("Cannot finish hashing the update fixture.");
  return value.get();
}

void write(const std::string& path, const std::vector<std::uint8_t>& bytes) {
  std::ofstream file{path, std::ios::binary | std::ios::trunc};
  file.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
  if (!file) throw std::runtime_error("Cannot write the update fixture.");
}

std::vector<std::uint8_t> read(const std::string& path) {
  std::ifstream file{path, std::ios::binary};
  if (!file) throw std::runtime_error("Cannot read the update fixture.");
  return {std::istreambuf_iterator<char>{file}, std::istreambuf_iterator<char>{}};
}

class TemporaryDirectory {
public:
  TemporaryDirectory() {
    char name[] = "/tmp/wodex-update-test-XXXXXX";
    char* created = ::mkdtemp(name);
    if (!created) throw std::runtime_error("Cannot create temporary update storage.");
    path_ = created;
  }
  ~TemporaryDirectory() { std::error_code error; std::filesystem::remove_all(path_, error); }
  std::string path(const std::string& name = "wodex.nro") const { return (path_ / name).string(); }
private:
  std::filesystem::path path_;
};

wodex::UpdateStatus wait(Updater& updater) {
  const auto deadline = Clock::now() + std::chrono::seconds{10};
  while (updater.active() && Clock::now() < deadline) std::this_thread::sleep_for(std::chrono::milliseconds{2});
  if (updater.active()) {
    updater.cancel();
    throw std::runtime_error("The mock update worker exceeded its deadline.");
  }
  const auto status = updater.status();
  check(status.phase != UpdatePhase::Checking && status.phase != UpdatePhase::Downloading,
    "The worker stopped without publishing a terminal status.");
  return status;
}

PendingUpdate fixture(const TemporaryDirectory& directory) {
  const auto old = nro(1);
  const auto next = nro(2);
  PendingUpdate pending{directory.path(), directory.path() + ".update.part", directory.path("wodex.previous.nro"), digest(next), next.size()};
  write(pending.target_path, old);
  write(pending.staged_path, next);
  return pending;
}

void test_download(const std::string& base) {
  TemporaryDirectory directory;
  const auto old = nro(1);
  write(directory.path(), old);
  std::optional<PendingUpdate> pending;
  {
    Updater updater{"0.2.0", directory.path(), base};
    updater.check("fixture/good");
    const auto available = wait(updater);
    check(available.phase == UpdatePhase::Available && available.version == "0.3.0", "A newer release was not offered.");
    check(available.notes == "Verified mock update\n包含字体", "Release notes were not retained as UTF-8.");
    updater.download();
    const auto ready = wait(updater);
    check(ready.phase == UpdatePhase::Ready && ready.downloaded == ready.total && ready.total == 4096,
      "The downloaded release was not fully verified.");
    pending = updater.pending_install();
    check(pending.has_value() && pending->staged_path == directory.path() + ".update.part", "No staged install was prepared.");
    check(read(directory.path()) == old, "Downloading changed the installed executable.");
    struct stat status{};
    check(::stat(pending->staged_path.c_str(), &status) == 0 && (status.st_mode & 0777) == 0600,
      "The staged update is not private to its user.");
    updater.cancel();
    check(updater.status().phase == UpdatePhase::Ready && updater.pending_install().has_value(),
      "Cancellation discarded a verified ready update.");
  }
  check(std::filesystem::exists(pending->staged_path), "Destroying a ready updater deleted its staged install.");
  wodex::install_update(*pending);
  check(read(directory.path()) == nro(2), "The new executable was not installed.");
  check(read(pending->backup_path) == old, "The previous executable was not retained after installation.");
  check(!std::filesystem::exists(pending->staged_path), "The staged path remained after installation.");
}

void test_check_modes(const std::string& base) {
  Updater current{"0.3.0", "", base};
  current.check("fixture/good");
  check(wait(current).phase == UpdatePhase::Current, "The current release was incorrectly offered as an upgrade.");
  Updater desktop{"0.2.0", "", base};
  desktop.check("fixture/good");
  check(wait(desktop).phase == UpdatePhase::Available, "Desktop could not inspect a newer release.");
  desktop.download();
  check(wait(desktop).phase == UpdatePhase::Error && !desktop.pending_install(), "An unknown executable path allowed replacement.");
  for (const std::string repository : {"", "fixture/../escape", "https://github.com/fixture/good", "fixture/good\r\nHeader: value", "bad_owner/repo", "owner--name/repo"}) {
    Updater invalid{"0.2.0", "", base};
    invalid.check(repository);
    check(wait(invalid).phase == UpdatePhase::Error, "An invalid repository was accepted.");
  }
  rejects([] { Updater invalid{"0.2.0", "", "http://example.com"}; }, "A non-loopback test server was accepted.");
  rejects([] { Updater invalid{"0.2.0", "", "http://user:password@127.0.0.1:9000"}; }, "A test server with credentials was accepted.");
  rejects([] { Updater invalid{"0.2.0", "", "http://127.0.0.1:9000/path"}; }, "A test server path was accepted as an origin.");
}

void test_failures(const std::string& base) {
  for (const std::string scenario : {"hash", "short", "long", "malformed", "aset", "no-assets", "zero-romfs", "romfs-header", "romfs-root", "romfs-data", "bad-redirect", "http-redirect", "credentials-redirect", "loop"}) {
    TemporaryDirectory directory;
    const auto original = nro(1);
    write(directory.path(), original);
    Updater updater{"0.2.0", directory.path(), base};
    updater.check("fixture/" + scenario);
    check(wait(updater).phase == UpdatePhase::Available, "The invalid download scenario did not publish a valid manifest.");
    updater.download();
    const auto failed = wait(updater);
    check(failed.phase == UpdatePhase::Error && !updater.pending_install(), "An invalid update was accepted for installation.");
    check(read(directory.path()) == original, "A failed download changed the installed executable.");
    check(!std::filesystem::exists(directory.path() + ".update.part"), "A failed download left an unverified partial file.");
    check(failed.message.find("private-update-body") == std::string::npos, "A provider error body reached the UI.");
  }
  for (const std::string scenario : {"http-error", "bad-json", "oversized-metadata", "wrong-repository"}) {
    Updater updater{"0.2.0", "", base};
    updater.check("fixture/" + scenario);
    const auto failed = wait(updater);
    check(failed.phase == UpdatePhase::Error && failed.message.find("private-update-body") == std::string::npos,
      "Invalid release metadata was accepted or leaked an error body.");
  }
}

void test_cancel(const std::string& base) {
  TemporaryDirectory directory;
  const auto original = nro(1);
  write(directory.path(), original);
  Updater updater{"0.2.0", directory.path(), base};
  updater.check("fixture/slow");
  check(wait(updater).phase == UpdatePhase::Available, "The cancellation fixture was not available.");
  updater.download();
  const auto deadline = Clock::now() + std::chrono::seconds{5};
  while (updater.active() && updater.status().downloaded == 0 && Clock::now() < deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds{2});
  }
  check(updater.active() && updater.status().downloaded > 0, "The cancellable download did not begin.");
  rejects([&] { updater.check("fixture/good"); }, "A second operation started during a download.");
  const auto cancelled_at = Clock::now();
  updater.cancel();
  check(Clock::now() - cancelled_at < std::chrono::milliseconds{100}, "Update cancellation blocked the UI thread.");
  check(wait(updater).phase == UpdatePhase::Cancelled && Clock::now() - cancelled_at < std::chrono::seconds{3},
    "The cancelled update did not stop promptly.");
  check(read(directory.path()) == original && !std::filesystem::exists(directory.path() + ".update.part"),
    "Cancellation changed the executable or retained a partial update.");
}

void test_paths_and_ready_reset(const std::string& base) {
  TemporaryDirectory directory;
  write(directory.path(), nro(1));
  for (const auto& target : {directory.path("../escape.nro"), directory.path("wodex.previous.nro"), std::string{"wodex.nro"}}) {
    Updater updater{"0.2.0", target, base};
    updater.check("fixture/good");
    check(wait(updater).phase == UpdatePhase::Available, "Path tests could not inspect the release.");
    updater.download();
    check(wait(updater).phase == UpdatePhase::Error, "An unsafe installation target was accepted.");
  }
  Updater updater{"0.2.0", directory.path(), base};
  updater.check("fixture/good");
  wait(updater);
  updater.download();
  check(wait(updater).phase == UpdatePhase::Ready, "The reset fixture did not become ready.");
  updater.check("fixture/good");
  check(wait(updater).phase == UpdatePhase::Available && !updater.pending_install() &&
    !std::filesystem::exists(directory.path() + ".update.part"), "A new check retained a stale pending install.");
}

int injected_rename(const char* from, const char* to) {
  ++rename_count;
  if (rename_count == 2 || (fail_rollback && rename_count == 3)) { errno = EIO; return -1; }
  return ::rename(from, to);
}

void test_install_safety() {
  {
    TemporaryDirectory directory;
    auto pending = fixture(directory);
    const auto original = nro(1);
    write(pending.backup_path, nro(7));
    auto tampered = nro(2);
    tampered.back() = 99;
    write(pending.staged_path, tampered);
    rejects([&] { wodex::install_update(pending); }, "A tampered staged executable was installed.");
    check(read(pending.target_path) == original && read(pending.backup_path) == nro(7),
      "Checksum failure changed an installed or backup executable.");
    write(pending.staged_path, nro(2));
    pending.staged_path = directory.path("unrelated.nro");
    rejects([&] { wodex::install_update(pending); }, "An unrelated staged path was accepted.");
    pending = fixture(directory);
    pending.backup_path = directory.path("unrelated-backup.nro");
    rejects([&] { wodex::install_update(pending); }, "An unrelated backup path was accepted.");
    pending = fixture(directory);
    ++pending.size;
    rejects([&] { wodex::install_update(pending); }, "A changed file size passed installation verification.");
  }
  {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    write(pending.backup_path, nro(7));
    auto invalid_current = nro(1);
    invalid_current[0x10] = 0;
    write(pending.target_path, invalid_current);
    rejects([&] { wodex::install_update(pending); }, "An invalid canonical NRO allowed removal of the previous version.");
    check(read(pending.backup_path) == nro(7) && read(pending.target_path) == invalid_current,
      "An invalid canonical NRO caused the last valid backup to be lost.");
    std::filesystem::remove(pending.target_path);
    rejects([&] { wodex::install_update(pending); }, "A missing canonical executable allowed installation.");
    check(read(pending.backup_path) == nro(7), "The missing-target failure removed a usable backup.");
  }
  {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    std::filesystem::create_directory(pending.backup_path);
    rejects([&] { wodex::install_update(pending); }, "A backup directory was removed during installation.");
    check(read(pending.target_path) == nro(1), "A backup-path failure changed the installed executable.");
  }
  for (const bool rollback_failure : {false, true}) {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    rename_count = 0;
    fail_rollback = rollback_failure;
    rejects([&] { wodex::detail::install_update_with_rename(pending, &injected_rename); },
      "An injected install rename failure did not reach the caller.");
    check(rename_count == 3 && read(pending.staged_path) == nro(2), "The rollback sequence did not retain the downloaded executable.");
    if (rollback_failure) {
      check(!std::filesystem::exists(pending.target_path) && read(pending.backup_path) == nro(1),
        "A rollback failure lost the manually launchable previous version.");
      rejects([&] { wodex::install_update(pending); }, "Retrying during a rename gap removed the previous version.");
      check(read(pending.backup_path) == nro(1), "A retry deleted the only installed-version backup.");
    } else {
      check(read(pending.target_path) == nro(1), "A failed second rename did not restore the installed executable.");
    }
  }
  {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    write(pending.backup_path, nro(7));
    wodex::install_update(pending);
    check(read(pending.target_path) == nro(2) && read(pending.backup_path) == nro(1),
      "A successful installation did not rotate the previous-version backup.");
  }
}

void test_real_artifact(const std::string& path) {
  TemporaryDirectory directory;
  const auto artifact = read(path);
  auto pending = fixture(directory);
  write(pending.staged_path, artifact);
  pending.size = artifact.size();
  pending.sha256 = digest(artifact);
  wodex::install_update(pending);
  check(digest(read(pending.target_path)) == pending.sha256, "The real packaged NRO failed full installation validation.");
  check(read(pending.backup_path) == nro(1), "The real NRO installation did not retain the previous executable.");
}

}  // namespace

int main(int argc, char** argv) {
  try {
    if (argc != 2 && argc != 3) throw std::runtime_error("Run this binary through tests/mock_updates.py.");
    test_download(argv[1]);
    test_check_modes(argv[1]);
    test_failures(argv[1]);
    test_cancel(argv[1]);
    test_paths_and_ready_reset(argv[1]);
    test_install_safety();
    if (argc == 3) test_real_artifact(argv[2]);
    std::cout << "Update integration: " << passed << " checks passed.\n";
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "Update integration failed: " << error.what() << '\n';
    return 1;
  }
}
