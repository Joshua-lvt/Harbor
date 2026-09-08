//go:build windows && !harborvpx

// Screen capture boundary on Windows without the native encoder tag: an
// honest refusal. Build with `-tags harborvpx` (plus a pkg-config-visible
// libvpx) for the real GDI capture in screen_windows_gdi.go.
//
// Capturing without an encoder would only produce frames nobody can send,
// so the refusal keeps share start on the existing capture_unavailable
// path (error.call.screenShareUnavailable) instead of shipping a black
// frame that pretends to be the user's screen. Voice calls are unaffected.
package main

import (
	"errors"
)

func openPlatformScreenCapture() (screenCapture, error) {
	return nil, errors.New("screen sharing needs a Windows build with -tags harborvpx")
}
