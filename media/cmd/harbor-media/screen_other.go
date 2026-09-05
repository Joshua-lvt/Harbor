//go:build !cgo || (!linux && !windows)

// Screen capture boundary on any other OS: an honest refusal, mirroring
// the Windows stub. Only platforms with a real adapter report captures.
package main

import (
	"errors"
)

func openPlatformScreenCapture() (screenCapture, error) {
	return nil, errors.New("screen sharing is not supported on this platform")
}
