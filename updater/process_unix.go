//go:build !windows

package updater

import (
	"errors"
	"os"
	"syscall"
	"time"
)

func waitPID(pid int, timeout time.Duration) (bool, error) {
	if pid <= 0 {
		return false, errors.New("parent PID must be positive")
	}
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		err := syscall.Kill(pid, 0)
		if err == nil {
			time.Sleep(25 * time.Millisecond)
			continue
		}
		if errors.Is(err, syscall.ESRCH) {
			return true, nil
		}
		return false, err
	}
	return false, nil
}

func processAlive(pid int) bool             { return pid > 0 && syscall.Kill(pid, 0) == nil }
func gracefulTerminate(p *os.Process) error { return p.Signal(os.Interrupt) }

func CaptureLaunchToken(int) (uintptr, error) { return 0, errors.New("launch tokens are Windows-only") }
func IsElevated() bool                        { return false }
func ProcessExecutable(int) (string, error) {
	return "", errors.New("process executable lookup is Windows-only")
}
