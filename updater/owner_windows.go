//go:build windows

package updater

func ownedByCurrentUser(path string) bool { return windowsPathOwnedByCurrentUser(path) }
