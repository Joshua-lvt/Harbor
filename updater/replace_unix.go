//go:build !windows

package updater

import "os"

func atomicReplace(from, to string) error         { return os.Rename(from, to) }
func syncReplacedFileDirectory(path string) error { return syncDirectory(path) }
