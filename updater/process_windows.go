//go:build windows

package updater

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"
	"unsafe"
)

var pDuplicateTokenEx = advapi.NewProc("DuplicateTokenEx")
var pQueryFullProcessImageName = kernel.NewProc("QueryFullProcessImageNameW")
var pOpenEvent = kernel.NewProc("OpenEventW")
var pSetEvent = kernel.NewProc("SetEvent")

const (
	processQueryLimitedInformation = 0x1000
	synchronize                    = 0x00100000
	waitObject0                    = 0
	waitTimeout                    = 258
	waitFailed                     = 0xffffffff
)

func openProcess(pid int) (syscall.Handle, error) {
	if pid <= 0 {
		return 0, errors.New("PID must be positive")
	}
	h, err := syscall.OpenProcess(processQueryLimitedInformation|synchronize, false, uint32(pid))
	if err != nil {
		return 0, err
	}
	return h, nil
}
func waitPID(pid int, timeout time.Duration) (bool, error) {
	h, err := openProcess(pid)
	if err != nil {
		if err == syscall.Errno(87) || err == syscall.Errno(2) {
			return true, nil
		}
		return false, err
	}
	defer syscall.CloseHandle(h)
	ms := uint32(timeout / time.Millisecond)
	if ms == 0 {
		ms = 1
	}
	r, err := syscall.WaitForSingleObject(h, ms)
	if err != nil {
		return false, err
	}
	if r == waitObject0 {
		return true, nil
	}
	if r == waitTimeout {
		return false, nil
	}
	if r == waitFailed {
		return false, syscall.EINVAL
	}
	return false, errors.New("unexpected process wait result")
}
func processAlive(pid int) bool {
	h, err := openProcess(pid)
	if err != nil {
		return false
	}
	defer syscall.CloseHandle(h)
	r, err := syscall.WaitForSingleObject(h, 0)
	return err == nil && r != waitObject0
}
func gracefulTerminate(p *os.Process) error { return p.Kill() }

func CaptureLaunchToken(pid int) (uintptr, error) {
	process, err := openProcess(pid)
	if err != nil {
		return 0, err
	}
	defer syscall.CloseHandle(process)
	var token uintptr
	if r, _, e := pOpen.Call(uintptr(process), 0x0001|0x0002|0x0008, uintptr(unsafe.Pointer(&token))); r == 0 {
		return 0, e
	}
	defer syscall.CloseHandle(syscall.Handle(token))
	var primary uintptr
	if r, _, e := pDuplicateTokenEx.Call(token, 0x02000000, 0, 2, 1, uintptr(unsafe.Pointer(&primary))); r == 0 {
		return 0, e
	}
	return primary, nil
}

func IsElevated() bool {
	var token uintptr
	current, _, _ := pCurrent.Call()
	if r, _, _ := pOpen.Call(current, 0x0008, uintptr(unsafe.Pointer(&token))); r == 0 {
		return false
	}
	defer syscall.CloseHandle(syscall.Handle(token))
	var elevation uint32
	var size uint32
	r, _, _ := pToken.Call(token, 20, uintptr(unsafe.Pointer(&elevation)), unsafe.Sizeof(elevation), uintptr(unsafe.Pointer(&size)))
	return r != 0 && elevation != 0
}

func ProcessExecutable(pid int) (string, error) {
	process, err := openProcess(pid)
	if err != nil {
		return "", err
	}
	defer syscall.CloseHandle(process)
	buffer := make([]uint16, 32768)
	size := uint32(len(buffer))
	if r, _, e := pQueryFullProcessImageName.Call(uintptr(process), 0, uintptr(unsafe.Pointer(&buffer[0])), uintptr(unsafe.Pointer(&size))); r == 0 {
		return "", e
	}
	return syscall.UTF16ToString(buffer[:size]), nil
}

func SignalReadyEvent(name string) error {
	const prefix = `Local\HarborUpdate-`
	if len(name) != len(prefix)+36 || name[:len(prefix)] != prefix || !ValidTransaction(name[len(prefix):]) {
		return errors.New("invalid readiness event")
	}
	value, err := syscall.UTF16PtrFromString(name)
	if err != nil {
		return err
	}
	handle, _, callErr := pOpenEvent.Call(0x0002, 0, uintptr(unsafe.Pointer(value)))
	if handle == 0 {
		return callErr
	}
	defer pClose.Call(handle)
	if result, _, callErr := pSetEvent.Call(handle); result == 0 {
		return callErr
	}
	return nil
}

func RelaunchAfterParent(pid int, program string, token uintptr) error {
	exited, err := waitPID(pid, 10*time.Second)
	if err != nil || !exited {
		if err != nil {
			return err
		}
		return errors.New("parent is still running")
	}
	command := exec.Command(program)
	command.Dir = filepath.Dir(program)
	configureLaunch(command, token)
	if err = command.Start(); err != nil {
		return err
	}
	return command.Process.Release()
}
