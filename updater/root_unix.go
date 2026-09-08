//go:build !windows

package updater

import (
	"errors"
	"os"
	"path/filepath"
)

func canonicalUpdaterRoot() (string, error) {
	cache, err := os.UserCacheDir()
	if err != nil || cache == "" {
		return "", errors.New("user cache directory is unavailable")
	}
	return filepath.Clean(filepath.Join(cache, "Harbor", "harbor-updater")), nil
}

func samePath(a, b string) bool { return filepath.Clean(a) == filepath.Clean(b) }
