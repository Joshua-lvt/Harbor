//go:build windows && harborvpx

// Real screen capture on Windows via GDI (no cgo, no new modules —
// golang.org/x/sys is already a dependency).
//
// The device context and bitmap are opened once: every frame re-blits the
// primary display into the same bitmap and reads it back as top-down
// 32-bit BGRA, re-laid as RGBA. A blit or read failure ends the share
// through the existing capture_unavailable path instead of shipping a
// stale or black frame.
package main

import (
	"errors"
	"fmt"
	"unsafe"

	"golang.org/x/sys/windows"
)

var (
	modUser32 = windows.NewLazySystemDLL("user32.dll")
	modGDI32  = windows.NewLazySystemDLL("gdi32.dll")

	procGetDC                  = modUser32.NewProc("GetDC")
	procReleaseDC              = modUser32.NewProc("ReleaseDC")
	procGetSystemMetrics       = modUser32.NewProc("GetSystemMetrics")
	procCreateCompatibleDC     = modGDI32.NewProc("CreateCompatibleDC")
	procCreateCompatibleBitmap = modGDI32.NewProc("CreateCompatibleBitmap")
	procSelectObject           = modGDI32.NewProc("SelectObject")
	procBitBlt                 = modGDI32.NewProc("BitBlt")
	procGetDIBits              = modGDI32.NewProc("GetDIBits")
	procDeleteObject           = modGDI32.NewProc("DeleteObject")
	procDeleteDC               = modGDI32.NewProc("DeleteDC")
)

const (
	smCxScreen   = 0
	smCyScreen   = 1
	srccopy      = 0x00CC0020
	biRGB        = 0
	dibRGBColors = 0
)

// bitmapInfoHeader is the 40-byte BITMAPINFOHEADER. A 32-bit BI_RGB bitmap
// carries no palette, so the header alone is the whole BITMAPINFO.
type bitmapInfoHeader struct {
	size          uint32
	width         int32
	height        int32 // negative height requests top-down rows
	planes        uint16
	bitCount      uint16
	compression   uint32
	sizeImage     uint32
	xPelsPerMeter int32
	yPelsPerMeter int32
	clrUsed       uint32
	clrImportant  uint32
}

// gdiScreenCapture owns one memory DC with a display-sized bitmap.
type gdiScreenCapture struct {
	memDC  uintptr
	bitmap uintptr
	oldObj uintptr
	width  int
	height int
	raw    []byte
	closed bool
}

// openPlatformScreenCapture opens the primary display. Multi-monitor
// spans are a later addition: the primary screen is the honest frame,
// never a stitched guess.
//
// Note: none of the GDI APIs below report through GetLastError, so only
// their return values are checked — the errno out-param is ignored.
func openPlatformScreenCapture() (screenCapture, error) {
	width, _, _ := procGetSystemMetrics.Call(smCxScreen)
	if width <= 0 {
		return nil, errors.New("screen width is unavailable")
	}
	height, _, _ := procGetSystemMetrics.Call(smCyScreen)
	if height <= 0 {
		return nil, errors.New("screen height is unavailable")
	}
	w, h := int(width), int(height)
	// I420 requires even dimensions; an odd edge case shrinks by one.
	w -= w % 2
	h -= h % 2
	if w <= 0 || h <= 0 {
		return nil, fmt.Errorf("screen size is unusable: %dx%d", w, h)
	}

	screenDC, _, _ := procGetDC.Call(0)
	if screenDC == 0 {
		return nil, errors.New("screen device context is unavailable")
	}
	memDC, _, _ := procCreateCompatibleDC.Call(screenDC)
	if memDC == 0 {
		procReleaseDC.Call(0, screenDC)
		return nil, errors.New("capture device context is unavailable")
	}
	bitmap, _, _ := procCreateCompatibleBitmap.Call(screenDC, uintptr(w), uintptr(h))
	procReleaseDC.Call(0, screenDC)
	if bitmap == 0 {
		procDeleteDC.Call(memDC)
		return nil, errors.New("capture bitmap is unavailable")
	}
	oldObj, _, _ := procSelectObject.Call(memDC, bitmap)
	if oldObj == 0 || oldObj == 0xFFFFFFFF {
		procDeleteObject.Call(bitmap)
		procDeleteDC.Call(memDC)
		return nil, errors.New("capture bitmap cannot be selected")
	}
	return &gdiScreenCapture{
		memDC:  memDC,
		bitmap: bitmap,
		oldObj: oldObj,
		width:  w,
		height: h,
		raw:    make([]byte, w*h*4),
	}, nil
}

func (c *gdiScreenCapture) Size() (width, height int) { return c.width, c.height }

// NextFrame re-blits the screen and returns a fresh RGBA frame. The
// internal buffer is reused across frames; the returned slice is a copy,
// so the pump can hold it while the next blit runs.
func (c *gdiScreenCapture) NextFrame() ([]byte, error) {
	if c.closed {
		return nil, errors.New("screen capture is closed")
	}
	screenDC, _, _ := procGetDC.Call(0)
	if screenDC == 0 {
		return nil, errors.New("screen device context is unavailable")
	}
	ok, _, _ := procBitBlt.Call(c.memDC, 0, 0, uintptr(c.width), uintptr(c.height),
		screenDC, 0, 0, srccopy)
	procReleaseDC.Call(0, screenDC)
	if ok == 0 {
		return nil, errors.New("screen blit failed")
	}
	header := bitmapInfoHeader{
		size:        uint32(unsafe.Sizeof(bitmapInfoHeader{})),
		width:       int32(c.width),
		height:      -int32(c.height), // top-down: no row flip needed
		planes:      1,
		bitCount:    32,
		compression: biRGB,
	}
	lines, _, _ := procGetDIBits.Call(c.memDC, c.bitmap, 0, uintptr(c.height),
		uintptr(unsafe.Pointer(&c.raw[0])), uintptr(unsafe.Pointer(&header)), dibRGBColors)
	if lines != uintptr(c.height) {
		return nil, errors.New("screen readback failed")
	}
	// BGRA (BGRX, alpha unused) to RGBA, alpha forced opaque.
	rgba := make([]byte, len(c.raw))
	for i := 0; i < len(c.raw); i += 4 {
		rgba[i] = c.raw[i+2]   // R
		rgba[i+1] = c.raw[i+1] // G
		rgba[i+2] = c.raw[i]   // B
		rgba[i+3] = 0xFF       // A
	}
	return rgba, nil
}

func (c *gdiScreenCapture) Close() {
	if c.closed {
		return
	}
	c.closed = true
	procSelectObject.Call(c.memDC, c.oldObj)
	procDeleteObject.Call(c.bitmap)
	procDeleteDC.Call(c.memDC)
}
