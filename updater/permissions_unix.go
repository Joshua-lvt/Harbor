//go:build !windows

package updater

import "os"

func securePrivatePath(path string) bool {
	i, e := os.Stat(path)
	return e == nil && i.Mode().Perm()&0077 == 0 && ownedByCurrentUser(path)
}
