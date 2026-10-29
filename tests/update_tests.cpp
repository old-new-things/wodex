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

std::vector<std::uint8_t> prefix(const std::vector<std::uint8_t>& bytes, std::size_t length) {
  if (length > bytes.size()) throw std::runtime_error("Invalid partial fixture length.");
  return {bytes.begin(), bytes.begin() + static_cast<std::ptrdiff_t>(length)};
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

void check_single_partial(const TemporaryDirectory& directory) {
  std::size_t partials = 0;
  for (const auto& entry : std::filesystem::directory_iterator(
      std::filesystem::path(directory.path()).parent_path())) {
    const auto name = entry.path().filename().string();
    check(name == "wodex.nro" || name == "wodex.nro.update.part",
      "Resume created a second temporary file or metadata sidecar.");
    if (name == "wodex.nro.update.part") ++partials;
  }
  check(partials <= 1, "More than one partial update was retained.");
}

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
  PendingUpdate pending{directory.path(), directory.path() + ".update.part", directory.path(".wodex.previous.nro"), digest(next), next.size()};
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
    check(available.phase == UpdatePhase::Available && available.version == "0.3.0", "A newer version was not offered.");
    check(available.notes == "Verified mock update\n包含字体", "Version notes were not retained as UTF-8.");
    updater.download();
    const auto ready = wait(updater);
    check(ready.phase == UpdatePhase::Ready && ready.downloaded == ready.total && ready.total == 4096,
      "The downloaded version was not fully verified.");
    pending = updater.pending_install();
    check(pending.has_value() && pending->staged_path == directory.path() + ".update.part", "No staged install was prepared.");
    check(pending->backup_path == directory.path(".wodex.previous.nro"),
      "The selected update would create a visible recovery NRO in hbmenu.");
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
  check(wait(current).phase == UpdatePhase::Current, "The current version was incorrectly offered as an upgrade.");
  Updater empty{"0.2.0", "", base};
  empty.check("fixture/empty");
  check(wait(empty).phase == UpdatePhase::Current, "An empty version channel offered an update.");
  Updater desktop{"0.2.0", "", base};
  desktop.check("fixture/good");
  check(wait(desktop).phase == UpdatePhase::Available, "Desktop could not inspect a newer version.");
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
  for (const std::string scenario : {"hash", "short", "long", "malformed", "aset", "no-assets", "zero-romfs", "romfs-header", "romfs-root", "romfs-data", "bad-redirect", "http-redirect", "credentials-redirect", "loop", "same-project-redirect", "cross-project-redirect"}) {
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
  for (const std::string scenario : {"http-error", "bad-json", "oversized-metadata", "wrong-repository", "wrong-owner", "manifest-redirect", "nightly-feed", "unknown-schema", "missing-checksum", "missing-build"}) {
    Updater updater{"0.2.0", "", base};
    updater.check("fixture/" + scenario);
    const auto failed = wait(updater);
    check(failed.phase == UpdatePhase::Error && failed.message.find("private-update-body") == std::string::npos,
      "Invalid version metadata was accepted or leaked an error body.");
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
  const auto partial = read(directory.path() + ".update.part");
  check(read(directory.path()) == original && !partial.empty() && partial.size() < 1024 * 1024,
    "Cancellation changed the executable or did not retain the partial update.");
  check(partial == prefix(nro(2, 1024 * 1024), partial.size()),
    "Cancellation retained bytes other than the downloaded artifact prefix.");
  check_single_partial(directory);
}

void test_resume_after_disconnect(const std::string& base) {
  for (const bool restart : {false, true}) {
    TemporaryDirectory directory;
    const auto original = nro(1);
    const auto next = nro(2);
    write(directory.path(), original);
    const auto scenario = restart ? "resume-disconnect" : "resume-retry";
    auto updater = std::make_unique<Updater>("0.2.0", directory.path(), base);
    updater->check(std::string{"fixture/"} + scenario);
    check(wait(*updater).phase == UpdatePhase::Available, "Resume fixture was not offered.");
    updater->download();
    check(wait(*updater).phase == UpdatePhase::Error && !updater->pending_install(),
      "Interrupted transfer became installable.");
    check(read(directory.path() + ".update.part") == prefix(next, 1024),
      "Interrupted transfer did not retain its exact prefix.");
    check(read(directory.path()) == original, "Interrupted transfer changed the installed executable.");
    check_single_partial(directory);
    if (restart) {
      updater.reset();
      check(read(directory.path() + ".update.part") == prefix(next, 1024),
        "Closing the updater discarded a resumable download.");
      updater = std::make_unique<Updater>("0.2.0", directory.path(), base);
      updater->check(std::string{"fixture/"} + scenario);
      check(wait(*updater).phase == UpdatePhase::Available, "Restart could not reselect the partial download.");
    }
    updater->download();
    const auto ready = wait(*updater);
    check(ready.phase == UpdatePhase::Ready && ready.downloaded == next.size() && ready.total == next.size(),
      "The resumed download did not become verified and complete.");
    check(updater->pending_install().has_value() && read(directory.path() + ".update.part") == next,
      "The resumed file differs from the advertised artifact.");
    check(read(directory.path()) == original, "Resuming replaced the installed executable before installation.");
    check_single_partial(directory);
  }
}

void test_existing_partial_and_resume_fallback(const std::string& base) {
  const auto next = nro(2);
  for (const std::string scenario : {"resume-valid", "resume-ignored", "resume-416", "resume-corrupt",
    "resume-complete", "resume-complete-corrupt", "resume-oversized", "resume-empty"}) {
    TemporaryDirectory directory;
    const auto original = nro(1);
    write(directory.path(), original);
    auto partial = scenario == "resume-empty" ? std::vector<std::uint8_t>{}
      : (scenario == "resume-complete" || scenario == "resume-complete-corrupt" || scenario == "resume-oversized")
        ? next : prefix(next, 1024);
    if (scenario == "resume-corrupt" || scenario == "resume-complete-corrupt") partial[0x90] ^= 0xff;
    if (scenario == "resume-oversized") partial.push_back(0x55);
    write(directory.path() + ".update.part", partial);
    Updater updater{"0.2.0", directory.path(), base};
    updater.check("fixture/" + scenario);
    check(wait(updater).phase == UpdatePhase::Available, "Existing-partial scenario was not offered.");
    check(read(directory.path() + ".update.part") == partial,
      "Checking for updates discarded the retained partial file.");
    updater.download();
    const auto ready = wait(updater);
    check(ready.phase == UpdatePhase::Ready && ready.downloaded == next.size(),
      "Existing partial or bounded full-download fallback did not become ready.");
    check(updater.pending_install().has_value() && read(directory.path() + ".update.part") == next,
      "Existing partial produced an incorrect staged artifact.");
    check(read(directory.path()) == original, "Resuming or restarting changed the installed executable.");
    check_single_partial(directory);
  }
}

void test_rejects_invalid_resume_headers(const std::string& base) {
  for (const std::string scenario : {"resume-bad-start", "resume-missing-range", "resume-duplicate-range", "resume-encoded"}) {
    TemporaryDirectory directory;
    const auto original = nro(1);
    const auto partial = prefix(nro(2), 1024);
    write(directory.path(), original);
    write(directory.path() + ".update.part", partial);
    Updater updater{"0.2.0", directory.path(), base};
    updater.check("fixture/" + scenario);
    check(wait(updater).phase == UpdatePhase::Available, "Invalid-range scenario was not offered.");
    updater.download();
    const auto failed = wait(updater);
    check(failed.phase == UpdatePhase::Error && !updater.pending_install(),
      "Invalid resume response became installable.");
    check(read(directory.path() + ".update.part") == partial,
      "Invalid resume headers allowed response bytes to append to the partial file.");
    check(read(directory.path()) == original, "Invalid resume response changed the installed executable.");
    check(failed.message.find("private-update-body") == std::string::npos,
      "An invalid resume response leaked server body text.");
    check_single_partial(directory);
  }
}

void test_partial_path_safety(const std::string& base) {
  for (const bool symlink : {false, true}) {
    TemporaryDirectory directory;
    const auto original = nro(1);
    write(directory.path(), original);
    const auto staged = directory.path() + ".update.part";
    const auto unrelated = directory.path("unrelated.bin");
    if (symlink) {
      write(unrelated, nro(7));
      std::filesystem::create_symlink(unrelated, staged);
    } else {
      std::filesystem::create_directory(staged);
    }
    Updater updater{"0.2.0", directory.path(), base};
    updater.check("fixture/resume-valid");
    check(wait(updater).phase == UpdatePhase::Available, "Partial-path fixture was not offered.");
    updater.download();
    check(wait(updater).phase == UpdatePhase::Error && !updater.pending_install(),
      "A nonregular partial file was accepted for resume.");
    check(read(directory.path()) == original, "A partial-path failure changed the installed executable.");
    check(symlink ? std::filesystem::is_symlink(staged) : std::filesystem::is_directory(staged),
      "A nonregular partial path was deleted or replaced.");
    if (symlink) check(read(unrelated) == nro(7), "Resuming followed a symlink and changed another file.");
  }
}

void test_paths_and_ready_reset(const std::string& base) {
  TemporaryDirectory directory;
  write(directory.path(), nro(1));
  for (const auto& target : {directory.path("../escape.nro"), directory.path("wodex.previous.nro"),
      directory.path(".wodex.previous.nro"), std::string{"wodex.nro"}}) {
    Updater updater{"0.2.0", target, base};
    updater.check("fixture/good");
    check(wait(updater).phase == UpdatePhase::Available, "Path tests could not inspect the version.");
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
    read(directory.path() + ".update.part") == nro(2), "A new check retained a pending install or discarded its reusable file.");
}

void test_backup_migration(const std::string& base) {
  {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    const auto legacy = directory.path("wodex.previous.nro");
    write(legacy, nro(7));
    {
      Updater updater{"0.2.0", directory.path(), base};
      check(updater.status().phase == UpdatePhase::Idle && !updater.active() && !updater.pending_install(),
        "Recovery migration started an update or made a staged file installable.");
      check(!std::filesystem::exists(legacy) && read(pending.backup_path) == nro(7),
        "Startup did not hide the existing recovery NRO while preserving its bytes.");
      check(read(pending.target_path) == nro(1) && read(pending.staged_path) == nro(2),
        "Recovery migration changed the installed executable or retained download.");
    }
    Updater reopened{"0.2.0", directory.path(), base};
    check(reopened.status().phase == UpdatePhase::Idle && read(pending.backup_path) == nro(7) &&
      !std::filesystem::exists(legacy), "Reopening Wodex did not preserve the migrated recovery copy.");
  }
  {
    TemporaryDirectory directory;
    const auto target = directory.path("custom.nro");
    const auto legacy = directory.path("custom.previous.nro");
    const auto hidden = directory.path(".custom.previous.nro");
    write(target, nro(1));
    write(legacy, nro(7));
    Updater updater{"0.2.0", target, base};
    check(updater.status().phase == UpdatePhase::Idle && !std::filesystem::exists(legacy) &&
      read(hidden) == nro(7) && read(target) == nro(1),
      "Recovery migration ignored the actual loader filename.");
  }
  for (const std::string name : {"wodex.previous.nro", ".wodex.previous.nro"}) {
    TemporaryDirectory directory;
    const auto target = directory.path(name);
    const auto legacy_name = std::filesystem::path(name).stem().string() + ".previous.nro";
    const auto legacy = directory.path(legacy_name);
    write(target, nro(1));
    write(legacy, nro(7));
    Updater updater{"0.2.0", target, base};
    check(read(target) == nro(1) && read(legacy) == nro(7) &&
      !std::filesystem::exists(directory.path("." + legacy_name)),
      "Launching a recovery NRO migrated files despite replacement being disabled.");
  }
  for (const bool symlink : {false, true}) {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    const auto legacy = directory.path("wodex.previous.nro");
    const auto unrelated = directory.path("unrelated.bin");
    if (symlink) {
      write(unrelated, nro(7));
      std::filesystem::create_symlink(unrelated, legacy);
    } else {
      std::filesystem::create_directory(legacy);
    }
    Updater updater{"0.2.0", directory.path(), base};
    const auto status = updater.status();
    check(status.phase == UpdatePhase::Error && !status.message.empty() && !updater.active(),
      "A nonregular legacy recovery path did not produce a nonfatal migration error.");
    check(!std::filesystem::exists(pending.backup_path) &&
      (symlink ? std::filesystem::is_symlink(legacy) : std::filesystem::is_directory(legacy)),
      "Recovery migration moved or deleted a nonregular legacy path.");
    check(read(pending.target_path) == nro(1) && read(pending.staged_path) == nro(2),
      "A recovery migration error changed an installed or staged executable.");
    if (symlink) check(read(unrelated) == nro(7), "Recovery migration changed a symlink's destination.");
  }
  for (int hidden_kind = 0; hidden_kind < 3; ++hidden_kind) {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    const auto legacy = directory.path("wodex.previous.nro");
    const auto unrelated = directory.path("unrelated.bin");
    write(legacy, nro(7));
    if (hidden_kind == 0) {
      write(pending.backup_path, nro(8));
    } else if (hidden_kind == 1) {
      std::filesystem::create_directory(pending.backup_path);
    } else {
      write(unrelated, nro(8));
      std::filesystem::create_symlink(unrelated, pending.backup_path);
    }
    Updater updater{"0.2.0", directory.path(), base};
    check(updater.status().phase == UpdatePhase::Error && read(legacy) == nro(7),
      "Recovery migration overwrote or silently discarded a conflicting backup.");
    check(hidden_kind == 0 ? read(pending.backup_path) == nro(8) : hidden_kind == 1 ?
      std::filesystem::is_directory(pending.backup_path) : std::filesystem::is_symlink(pending.backup_path),
      "Recovery migration replaced the occupied hidden backup path.");
    if (hidden_kind == 2) check(read(unrelated) == nro(8), "A backup collision changed a symlink's destination.");
    check(read(pending.target_path) == nro(1) && read(pending.staged_path) == nro(2),
      "A backup collision changed the current executable or retained download.");
  }
}

int injected_rename(const char* from, const char* to) {
  ++rename_count;
  if (rename_count == 2 || (fail_rollback && rename_count == 3)) { errno = EIO; return -1; }
  return ::rename(from, to);
}

void test_install_safety() {
  {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    const auto legacy = directory.path("wodex.previous.nro");
    write(legacy, nro(7));
    wodex::install_update(pending);
    check(!std::filesystem::exists(legacy) && read(pending.backup_path) == nro(1) &&
      read(pending.target_path) == nro(2), "Direct installation left a visible legacy recovery copy.");
  }
  {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    const auto legacy = directory.path("wodex.previous.nro");
    write(legacy, nro(7));
    write(pending.backup_path, nro(8));
    rejects([&] { wodex::install_update(pending); }, "A recovery-backup collision allowed installation.");
    check(read(legacy) == nro(7) && read(pending.backup_path) == nro(8) &&
      read(pending.target_path) == nro(1) && read(pending.staged_path) == nro(2),
      "Installation changed files before resolving a recovery-backup collision.");
  }
  for (const bool invalid_current : {false, true}) {
    TemporaryDirectory directory;
    const auto pending = fixture(directory);
    const auto legacy = directory.path("wodex.previous.nro");
    write(legacy, nro(7));
    auto invalid = nro(invalid_current ? 1 : 2);
    invalid[0x10] = 0;
    write(invalid_current ? pending.target_path : pending.staged_path, invalid);
    rejects([&] { wodex::install_update(pending); },
      "An invalid executable allowed installation with a legacy recovery copy.");
    check(read(legacy) == nro(7) && !std::filesystem::exists(pending.backup_path),
      "Installation migrated recovery files before validating both executables.");
  }
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
        "A rollback failure lost the recoverable hidden previous version.");
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

void test_nightly_channel(const std::string& base) {
  using wodex::UpdateChannel;
  TemporaryDirectory directory;
  const auto original = nro(1);
  write(directory.path(), original);
  Updater updater{"0.3.0", directory.path(), base};
  updater.check("fixture/nightly-good", UpdateChannel::Nightly);
  const auto available = wait(updater);
  check(available.phase == UpdatePhase::Available, "The optional nightly channel did not offer a same-version build.");
  check(available.version == "0.3.0 nightly 20261020T010203Z-abcdef0", "The nightly identity was not displayed.");
  updater.download();
  check(wait(updater).phase == UpdatePhase::Ready && updater.pending_install().has_value(),
    "The nightly payload did not pass verification.");
  check(read(directory.path()) == original, "Checking or downloading nightly replaced the running executable.");
  updater.check("fixture/nightly-good", UpdateChannel::Version);
  check(wait(updater).phase == UpdatePhase::Current && !updater.pending_install() &&
    read(directory.path() + ".update.part") == nro(2),
    "Switching channels retained a pending install or discarded the single retained download.");
  updater.check("fixture/nightly-good", UpdateChannel::Nightly);
  check(wait(updater).phase == UpdatePhase::Available, "A repeat nightly check failed after changing channels.");
  Updater newer{"0.4.0", "", base};
  newer.check("fixture/nightly-good", UpdateChannel::Nightly);
  check(wait(newer).phase == UpdatePhase::Available, "A development build could not select a nightly with a lower base version.");
  newer.check("fixture/nightly-good", UpdateChannel::Version);
  check(wait(newer).phase == UpdatePhase::Current, "A development build selected a lower Versions build.");
  Updater empty{"0.3.0", "", base};
  empty.check("fixture/nightly-empty", UpdateChannel::Nightly);
  check(wait(empty).phase == UpdatePhase::Current, "An empty nightly channel offered an update.");
  for (const std::string scenario : {"nightly-bad-id", "nightly-bad-date", "nightly-missing-commit",
    "nightly-wrong-path", "nightly-wrong-channel"}) {
    Updater invalid{"0.3.0", "", base};
    invalid.check("fixture/" + scenario, UpdateChannel::Nightly);
    check(wait(invalid).phase == UpdatePhase::Error, "An invalid nightly feed was accepted.");
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
    test_nightly_channel(argv[1]);
    test_failures(argv[1]);
    test_cancel(argv[1]);
    test_resume_after_disconnect(argv[1]);
    test_existing_partial_and_resume_fallback(argv[1]);
    test_rejects_invalid_resume_headers(argv[1]);
    test_partial_path_safety(argv[1]);
    test_paths_and_ready_reset(argv[1]);
    test_backup_migration(argv[1]);
    test_install_safety();
    if (argc == 3) test_real_artifact(argv[2]);
    std::cout << "Update integration: " << passed << " checks passed.\n";
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "Update integration failed: " << error.what() << '\n';
    return 1;
  }
}
