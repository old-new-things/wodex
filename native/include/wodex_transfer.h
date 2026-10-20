#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Socket handles are nonblocking descriptors. Socket operations return -1 on
// error, -2 when retrying would be appropriate, or a nonnegative result.
// IPv4 addresses are four octets in network order. listen selects an unused
// port and writes it in host byte order. Buffers are borrowed only during calls.
typedef struct WodexTransferApi {
  bool (*local_ipv4)(uint8_t *);
  bool (*random)(uint8_t *, size_t);
  int32_t (*listen)(const uint8_t *, uint16_t *);
  int32_t (*accept)(int32_t);
  int32_t (*receive)(int32_t, uint8_t *, size_t);
  int32_t (*send)(int32_t, const uint8_t *, size_t);
  // 1 = ready, 0 = timed out, -1 = error. writable selects POLLOUT vs POLLIN.
  int32_t (*wait)(int32_t, bool, uint32_t);
  void (*close)(int32_t);
  uint64_t (*now_ms)(void);
  // Send FIN after queued response bytes while retaining the read half.
  bool (*shutdown_write)(int32_t);
} WodexTransferApi;

const WodexTransferApi *wodex_transfer_api(void);

#ifdef __cplusplus
}
#endif
