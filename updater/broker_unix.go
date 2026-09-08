//go:build !windows

package updater

import "errors"

func RegisteredInstallDir() (string, error) {
	return "", errors.New("registered installation is Windows-only")
}
