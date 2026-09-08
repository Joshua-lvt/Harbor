// Shared C memory helpers for the media worker (every cgo platform).
//
// This file carries no third-party includes — only the C standard library —
// so it builds wherever a C toolchain exists, including Windows with
// MinGW-w64. Platform C APIs live behind build tags: libvpx encoding in
// adapters_vpx_cgo.go and X11 capture in adapters_x11_cgo.go (both Linux).
package main

/*
#include <stdlib.h>
#include <string.h>

static void harborFreeBuffer(void *p) {
    free(p);
}
static void harborCopyFrame(unsigned char *dst, const unsigned char *src, size_t n) {
    memcpy(dst, src, n);
}
*/
import "C"

import (
	"unsafe"
)

// ---- audio device ids ----

// harborFreeCBytes releases a buffer the malgo binding allocated through
// C.CBytes — the device-id pointers handed to InitDevice, which copies them
// during ma_device_init and never retains the caller's allocation.
func harborFreeCBytes(pointer unsafe.Pointer) {
	C.harborFreeBuffer(pointer)
}
