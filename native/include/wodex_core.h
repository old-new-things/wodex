#ifndef WODEX_CORE_H
#define WODEX_CORE_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The core and its returned strings are owned exclusively by Rust. All calls
 * on one core must occur on the same thread, or be externally synchronized.
 * Input strings must be valid, NUL-terminated UTF-8. NULL saved_json starts a
 * fresh session. Invalid saved data is exposed in snapshot.state.error and
 * prevents export until an explicit reset_storage command is issued.
 * Each returned string is JSON and must be released with wodex_string_free.
 * A NULL core returns NULL. Invalid commands produce an error JSON object.
 * Allocation failure follows the runtime's abort behavior.
 * Do not retain input pointers: their bytes are copied during each call. */
typedef struct WodexCore WodexCore;

WodexCore* wodex_new(const char* saved_json);
void wodex_free(WodexCore* core);
char* wodex_call(WodexCore* core, const char* command_json);
char* wodex_feed(WodexCore* core, const uint8_t* data, size_t len);
void wodex_string_free(char* value);

/* Inspect bounded public GitHub release metadata. No network operation occurs.
 * Returns JSON: {ok, available, release?: {version, download_url, size, sha256,
 * notes}}. Only newer stable versions with an uploaded wodex.nro asset and a
 * SHA-256 digest are accepted. Returned strings use wodex_string_free. */
char* wodex_update_inspect(const char* release_json, const char* current_version);

/* Streaming SHA-256. finish returns an owned lowercase hexadecimal string and
 * does not free the hash object. Repeated finish calls return the same digest;
 * updates after finish return false. NULL data is allowed only for zero length.
 * Feed at most 1 MiB per call and 128 MiB total. */
typedef struct WodexSha256 WodexSha256;
WodexSha256* wodex_sha256_new(void);
bool wodex_sha256_update(WodexSha256* hash, const uint8_t* data, size_t len);
char* wodex_sha256_finish(WodexSha256* hash);
void wodex_sha256_free(WodexSha256* hash);

/* Validate an NRO file prefix of at least 0x80 bytes against the full file size.
 * Checks NRO0 code/segment bounds. If the prefix includes an ASET header at the
 * end of the code, also validates its version and asset bounds. When the ASET
 * lies outside the prefix the caller must inspect it separately by seeking.
 * File size must be between 0x80 bytes and 128 MiB. */
bool wodex_nro_validate(const uint8_t* header, size_t header_len, uint64_t file_size);

#ifdef __cplusplus
}
#endif

#endif
