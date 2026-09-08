//go:build !windows

package updater

import (
	"os"
	"syscall"
)

func ownedByCurrentUser(path string) bool {
	i, e := os.Stat(path)
	if e != nil {
		return false
	}
	st, ok := i.Sys().(*syscall.Stat_t)
	return ok && st.Uid == uint32(os.Getuid())
}
