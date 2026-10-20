#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { const uint8_t* data; size_t len; bool present; } WodexUrlPart;
typedef size_t (*WodexHttpReceive)(void*, const uint8_t*, size_t);
typedef bool (*WodexHttpProgress)(void*);
typedef struct {
  bool (*init)(void);
  void* (*create)(void);
  void (*destroy)(void*);
  bool (*set_string)(void*, uint32_t, const char*);
  bool (*set_number)(void*, uint32_t, int64_t);
  bool (*set_body)(void*, const uint8_t*, size_t);
  bool (*append_header)(void*, const char*);
  bool (*callbacks)(void*, void*, WodexHttpReceive, WodexHttpReceive, WodexHttpProgress);
  uint32_t (*perform)(void*);
  uint16_t (*status)(void*);
  void* (*url_parse)(const char*, const char*);
  WodexUrlPart (*url_part)(void*, uint32_t);
  void (*url_free)(void*);
} WodexHttpApi;
const WodexHttpApi* wodex_http_api(void);
#ifdef __cplusplus
}
#endif
