//go:build windows

package updater

import (
	"os/exec"
	"syscall"
)

func configureLaunch(command *exec.Cmd, token uintptr) {
	if token != 0 {
		command.SysProcAttr = &syscall.SysProcAttr{Token: syscall.Token(token)}
	}
}
