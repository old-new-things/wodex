#ifndef WODEX_CORE_H
#define WODEX_CORE_H

#include <stddef.h>
#include <stdint.h>

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

#ifdef __cplusplus
}
#endif

#endif
