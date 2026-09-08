//go:build windows

package updater

import (
	"syscall"
	"unsafe"
)

var pCreate = kernel.NewProc("CreateFileW")
var pFlush = kernel.NewProc("FlushFileBuffers")
var pClose = kernel.NewProc("CloseHandle")

func syncDirectory(path string) error {
	p, _ := syscall.UTF16PtrFromString(path)
	h, _, e := pCreate.Call(uintptr(unsafe.Pointer(p)), genericWrite, 1|2|4, 0, 3, 0x02000000, 0)
	if h == ^uintptr(0) {
		return e
	}
	defer pClose.Call(h)
	r, _, e := pFlush.Call(h)
	if r == 0 {
		return e
	}
	return nil
}
