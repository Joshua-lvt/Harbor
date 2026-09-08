// C ABI of the in-process Harbor core (implemented in
// core/harbor-core/src/app/mod.rs). The desktop binary drives the same
// state over framed stdio; Android cannot spawn child processes, so the
// mobile facade calls in directly. Same frames, same protocol version.
#pragma once

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct HarborCoreState HarborCoreState;

// Creates the core for `state_dir` (UTF-8). Null/empty falls back to the
// platform default. Null on catastrophic failure only.
HarborCoreState *harbor_core_create(const char *state_dir);
// Sets the absolute app-private harbor-media executable path. Must be called
// before the first call; returns false for null, relative, or live-call paths.
bool harbor_core_set_media_worker(HarborCoreState *handle, const char *worker_path);
// Destroys a handle. Null is a no-op.
void harbor_core_destroy(HarborCoreState *handle);
// Dispatches one framed request; returns concatenated framed replies
// (response plus pending events) with its length in `out_len`. Null with
// `out_len = 0` on any rejection; the caller logs and keeps running.
uint8_t *harbor_core_dispatch(HarborCoreState *handle, const uint8_t *request,
                              size_t request_len, size_t *out_len);
// One background step (signaling poll, link pump); pending events out.
// The host calls this about once a second, like the desktop pump.
uint8_t *harbor_core_tick(HarborCoreState *handle, size_t *out_len);
// Releases a buffer from dispatch/tick. Exactly one call per buffer.
void harbor_core_free(uint8_t *buffer, size_t len);

#ifdef __cplusplus
}
#endif
