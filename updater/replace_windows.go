//go:build windows

package updater

import (
	"syscall"
	"unsafe"
)

var kernel32 = syscall.NewLazyDLL("kernel32.dll")
var moveFileEx = kernel32.NewProc("MoveFileExW")

func atomicReplace(from, to string) error {
	f, _ := syscall.UTF16PtrFromString(from)
	t, _ := syscall.UTF16PtrFromString(to)
	r, _, e := moveFileEx.Call(uintptr(unsafe.Pointer(f)), uintptr(unsafe.Pointer(t)), 1|8)
	if r == 0 {
		return e
	}
	return nil
}

// atomicReplace uses MOVEFILE_WRITE_THROUGH, so it already waits for the
// replacement to reach disk and does not depend on directory flushing.
func syncReplacedFileDirectory(string) error { return nil }
