//go:build linux && !android

// X11 screen capture for the share pipeline (Linux with cgo).
//
// The Xlib helpers exist because common Xlib predicates (XRootWindow,
// XDisplayWidth, AllPlanes) are preprocessor macros, which cgo cannot call;
// each is resolved inside a tiny C shim here.
package main

/*
#cgo pkg-config: x11
#include <stdlib.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>

static Display *harborOpenDisplay(void) {
    return XOpenDisplay(NULL);
}
static Window harborRootWindow(Display *dpy) {
    return XRootWindow(dpy, DefaultScreen(dpy));
}
static int harborDisplayWidth(Display *dpy) {
    return XDisplayWidth(dpy, DefaultScreen(dpy));
}
static int harborDisplayHeight(Display *dpy) {
    return XDisplayHeight(dpy, DefaultScreen(dpy));
}
static XImage *harborGetImage(Display *dpy, Window root, int width, int height) {
    return XGetImage(dpy, root, 0, 0, width, height, AllPlanes, ZPixmap);
}
static void harborDestroyImage(XImage *image) {
    XDestroyImage(image);
}
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// ---- X11 capture ----

// x11Display is an open Xlib connection to the local session.
type x11Display = *C.Display

func x11Open() (x11Display, error) {
	display := C.harborOpenDisplay()
	if display == nil {
		return nil, fmt.Errorf("no X11 display is available for capture")
	}
	return display, nil
}

func x11ScreenSize(display x11Display) (int, int, error) {
	C.XSync(display, C.False)
	width := int(C.harborDisplayWidth(display))
	height := int(C.harborDisplayHeight(display))
	if width <= 0 || height <= 0 {
		return 0, 0, fmt.Errorf("screen geometry is unavailable")
	}
	return width, height, nil
}

// x11CaptureRGBA grabs the root window and re-lays its BGRX bytes out as
// RGBA. Anything but a 32-bit ZPixmap layout is refused rather than guessed.
func x11CaptureRGBA(display x11Display, width, height int) ([]byte, error) {
	image := C.harborGetImage(display, C.harborRootWindow(display), C.int(width), C.int(height))
	if image == nil {
		return nil, fmt.Errorf("screen capture failed")
	}
	defer C.harborDestroyImage(image)
	if image.bits_per_pixel != 32 || image.byte_order != C.LSBFirst {
		return nil, fmt.Errorf("unsupported XImage layout for capture")
	}
	size := width * height * 4
	raw := unsafe.Slice((*byte)(unsafe.Pointer(image.data)), size)
	rgba := make([]byte, size)
	for i := 0; i < size; i += 4 {
		rgba[i] = raw[i+2]   // R
		rgba[i+1] = raw[i+1] // G
		rgba[i+2] = raw[i]   // B
		rgba[i+3] = 0xFF     // A
	}
	return rgba, nil
}

func x11Close(display x11Display) {
	C.XCloseDisplay(display)
}
