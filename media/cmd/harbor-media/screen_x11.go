//go:build linux && cgo && !android

// Real screen capture on X11 sessions (Linux with cgo).
package main

import (
	"fmt"
)

// x11ScreenCapture captures the root window of a real X11 session.
type x11ScreenCapture struct {
	handle x11Display
	width  int
	height int
}

// openPlatformScreenCapture opens the X11 root window after the shared
// Wayland refusal in openScreenCapture already ruled out sessions where
// the root window would lie.
func openPlatformScreenCapture() (screenCapture, error) {
	handle, err := x11Open()
	if err != nil {
		return nil, err
	}
	width, height, err := x11ScreenSize(handle)
	if err != nil || width <= 0 || height <= 0 {
		x11Close(handle)
		return nil, fmt.Errorf("screen size is unavailable: %w", err)
	}
	// I420 requires even dimensions; an odd display edge case shrinks by one.
	return &x11ScreenCapture{
		handle: handle,
		width:  width - width%2,
		height: height - height%2,
	}, nil
}

func (c *x11ScreenCapture) Size() (int, int) { return c.width, c.height }

func (c *x11ScreenCapture) NextFrame() ([]byte, error) {
	return x11CaptureRGBA(c.handle, c.width, c.height)
}

func (c *x11ScreenCapture) Close() {
	x11Close(c.handle)
}
