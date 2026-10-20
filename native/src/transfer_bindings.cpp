#include "wodex_transfer.h"

#include <arpa/inet.h>
#include <cerrno>
#include <climits>
#include <cstring>
#include <fcntl.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#ifdef __SWITCH__
#include <switch.h>
#else
#include <ifaddrs.h>
#include <net/if.h>
#include <cstdlib>
#endif

namespace {
int32_t result(int value) noexcept {
  if (value >= 0) return value;
  return errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR ? -2 : -1;
}

bool configure(int fd) noexcept {
  const int flags = fcntl(fd, F_GETFL, 0);
  if (flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0) return false;
#ifdef SO_NOSIGPIPE
  const int enabled = 1;
  if (setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &enabled, sizeof(enabled)) < 0) return false;
#endif
  return true;
}

bool local_ipv4(uint8_t* output) noexcept {
#ifdef __SWITCH__
  uint32_t address = 0;
  if (R_FAILED(nifmGetCurrentIpAddress(&address))) return false;
  std::memcpy(output, &address, 4);
  return address != 0 && output[0] != 127 && output[0] < 224;
#else
  ifaddrs* interfaces = nullptr;
  if (getifaddrs(&interfaces) != 0) return false;
  bool found = false;
  for (auto* entry = interfaces; entry; entry = entry->ifa_next) {
    if (!entry->ifa_addr || entry->ifa_addr->sa_family != AF_INET ||
        !(entry->ifa_flags & IFF_UP) || (entry->ifa_flags & IFF_LOOPBACK)) continue;
    const auto* address = reinterpret_cast<const sockaddr_in*>(entry->ifa_addr);
    uint8_t candidate[4];
    std::memcpy(candidate, &address->sin_addr, sizeof(candidate));
    if (candidate[0] == 0 || candidate[0] == 127 || candidate[0] >= 224) continue;
    if (!found) {
      std::memcpy(output, candidate, sizeof(candidate));
      found = true;
    }
    // Prefer a LAN address over a public interface or a VPN's link-local route.
    if (candidate[0] == 10 || (candidate[0] == 172 && candidate[1] >= 16 && candidate[1] <= 31) ||
        (candidate[0] == 192 && candidate[1] == 168)) {
      std::memcpy(output, candidate, sizeof(candidate));
      break;
    }
  }
  freeifaddrs(interfaces);
  return found;
#endif
}

bool random_bytes(uint8_t* output, size_t size) noexcept {
#ifdef __SWITCH__
  if (R_FAILED(csrngInitialize())) return false;
  const Result outcome = csrngGetRandomBytes(output, size);
  csrngExit();
  return R_SUCCEEDED(outcome);
#elif defined(__APPLE__) || defined(__FreeBSD__) || defined(__OpenBSD__)
  arc4random_buf(output, size);
  return true;
#else
  const int fd = open("/dev/urandom", O_RDONLY | O_CLOEXEC);
  if (fd < 0) return false;
  size_t position = 0;
  while (position < size) {
    const ssize_t count = read(fd, output + position, size - position);
    if (count > 0) position += static_cast<size_t>(count);
    else if (count < 0 && errno == EINTR) continue;
    else break;
  }
  close(fd);
  return position == size;
#endif
}

int32_t listen_socket(const uint8_t* address, uint16_t* port) noexcept {
  const int fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
  if (fd < 0) return -1;
  sockaddr_in local{};
  local.sin_family = AF_INET;
  std::memcpy(&local.sin_addr, address, 4);
  socklen_t size = sizeof(local);
  if (!configure(fd) ||
      bind(fd, reinterpret_cast<const sockaddr*>(&local), sizeof(local)) < 0 ||
      getsockname(fd, reinterpret_cast<sockaddr*>(&local), &size) < 0 ||
      listen(fd, 2) < 0) {
    close(fd);
    return -1;
  }
  *port = ntohs(local.sin_port);
  return fd;
}

int32_t accept_socket(int32_t listener) noexcept {
  const int fd = accept(listener, nullptr, nullptr);
  if (fd < 0) return result(fd);
  if (!configure(fd)) {
    close(fd);
    return -1;
  }
  return fd;
}

int32_t receive(int32_t fd, uint8_t* output, size_t size) noexcept {
  if (size > INT_MAX) return -1;
  return result(static_cast<int>(recv(fd, output, size, 0)));
}

int32_t send_bytes(int32_t fd, const uint8_t* bytes, size_t size) noexcept {
  if (size > INT_MAX) return -1;
#ifdef MSG_NOSIGNAL
  constexpr int flags = MSG_NOSIGNAL;
#else
  constexpr int flags = 0;
#endif
  return result(static_cast<int>(send(fd, bytes, size, flags)));
}

int32_t wait_socket(int32_t fd, bool writable, uint32_t milliseconds) noexcept {
  pollfd descriptor{};
  descriptor.fd = fd;
  descriptor.events = writable ? POLLOUT : POLLIN;
  const int outcome = poll(&descriptor, 1, static_cast<int>(milliseconds > INT_MAX ? INT_MAX : milliseconds));
  if (outcome < 0) return errno == EINTR ? 0 : -1;
  if (outcome == 0) return 0;
  // A readable descriptor may also carry POLLHUP; let recv consume the bytes.
  if (descriptor.revents & descriptor.events) return 1;
  return descriptor.revents & (POLLERR | POLLHUP | POLLNVAL) ? -1 : 0;
}

void close_socket(int32_t fd) noexcept { close(fd); }
bool shutdown_write(int32_t fd) noexcept { return shutdown(fd, SHUT_WR) == 0; }

uint64_t now_ms() noexcept {
  timespec time{};
  if (clock_gettime(CLOCK_MONOTONIC, &time) != 0) return 0;
  return static_cast<uint64_t>(time.tv_sec) * 1000 + static_cast<uint64_t>(time.tv_nsec) / 1000000;
}
}

extern "C" const WodexTransferApi* wodex_transfer_api(void) {
  static const WodexTransferApi api = {
    local_ipv4, random_bytes, listen_socket, accept_socket, receive,
    send_bytes, wait_socket, close_socket, now_ms, shutdown_write
  };
  return &api;
}
