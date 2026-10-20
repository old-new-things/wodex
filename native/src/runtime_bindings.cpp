#include "wodex_runtime.h"
#include <mutex>
#include <thread>

namespace {
void* spawn(void (*entry)(void*), void* context) noexcept {
  try { return new std::thread(entry, context); } catch (...) { return nullptr; }
}
void join(void* handle) noexcept {
  auto* worker = static_cast<std::thread*>(handle);
  if (worker) { worker->join(); delete worker; }
}
void* mutex_new() noexcept {
  try { return new std::mutex; } catch (...) { return nullptr; }
}
void mutex_lock(void* handle) noexcept { static_cast<std::mutex*>(handle)->lock(); }
void mutex_unlock(void* handle) noexcept { static_cast<std::mutex*>(handle)->unlock(); }
void mutex_free(void* handle) noexcept { delete static_cast<std::mutex*>(handle); }
}
extern "C" const WodexRuntimeApi* wodex_runtime_api(void) {
  static const WodexRuntimeApi api = {spawn, join, mutex_new, mutex_lock, mutex_unlock, mutex_free};
  return &api;
}
