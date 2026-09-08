//go:build !windows

package updater

import "os/exec"

func configureLaunch(*exec.Cmd, uintptr) {}
