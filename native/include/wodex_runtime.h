#pragma once
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif
typedef struct {
  void* (*thread_spawn)(void (*entry)(void*), void* context);
  void (*thread_join)(void* handle);
  void* (*mutex_new)(void);
  void (*mutex_lock)(void* handle);
  void (*mutex_unlock)(void* handle);
  void (*mutex_free)(void* handle);
} WodexRuntimeApi;
const WodexRuntimeApi* wodex_runtime_api(void);
#ifdef __cplusplus
}
#endif
