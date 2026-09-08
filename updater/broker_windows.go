//go:build windows

package updater

import (
	"errors"
	"path/filepath"
	"syscall"
	"unsafe"
)

var pRegGetValue = advapi.NewProc("RegGetValueW")

func RegisteredInstallDir() (string, error) {
	subkey, _ := syscall.UTF16PtrFromString(`Software\Harbor`)
	value, _ := syscall.UTF16PtrFromString("InstallDir")
	var size uint32
	const hkeyLocalMachine = uintptr(0x80000002)
	const rrfRTRegSZ = 0x00000002
	if r, _, _ := pRegGetValue.Call(hkeyLocalMachine, uintptr(unsafe.Pointer(subkey)), uintptr(unsafe.Pointer(value)), rrfRTRegSZ, 0, 0, uintptr(unsafe.Pointer(&size))); r != 0 || size < 2 {
		return "", errors.New("registered Harbor installation not found")
	}
	buffer := make([]uint16, size/2)
	if r, _, _ := pRegGetValue.Call(hkeyLocalMachine, uintptr(unsafe.Pointer(subkey)), uintptr(unsafe.Pointer(value)), rrfRTRegSZ, 0, uintptr(unsafe.Pointer(&buffer[0])), uintptr(unsafe.Pointer(&size))); r != 0 {
		return "", errors.New("could not read registered Harbor installation")
	}
	return filepath.Clean(syscall.UTF16ToString(buffer)), nil
}
