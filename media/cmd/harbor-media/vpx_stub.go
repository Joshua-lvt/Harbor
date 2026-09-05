//go:build !cgo || android || (!linux && (!windows || !harborvpx))

// Honest video-encoder stub for platforms without the libvpx boundary
// (Windows and every other non-Linux OS).
//
// Voice calls do not need this encoder; only screen sharing does, and share
// start already maps the failure onto capture_unavailable
// (error.call.screenShareUnavailable), the same refusal a Wayland session
// gets. A native Windows encoder is a later, deliberate addition —
// inventing frames here would be worse than refusing.
package main

import (
	"errors"
	"fmt"
)

// errVPXUnavailable marks the honest "no encoder on this platform"
// failure so tests can tell it apart from real regressions.
var errVPXUnavailable = errors.New("video encoder is unavailable on this platform")

// vpxEncoder mirrors the Linux type's surface so the share pipeline
// compiles everywhere; every method reports unavailability.
type vpxEncoder struct {
	width  int
	height int
}

func newVPXEncoder(width, height int) (*vpxEncoder, error) {
	if width <= 0 || height <= 0 || width%2 != 0 || height%2 != 0 {
		return nil, fmt.Errorf("capture size must be even and positive")
	}
	return nil, errVPXUnavailable
}

func (e *vpxEncoder) encode(_ []byte, _ bool) ([]byte, error) {
	return nil, errVPXUnavailable
}

func (e *vpxEncoder) Close() {}
