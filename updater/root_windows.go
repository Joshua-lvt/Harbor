//go:build windows

package updater

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
)

func canonicalUpdaterRoot() (string, error) {
	cache, err := os.UserCacheDir()
	if err != nil || cache == "" {
		return "", errors.New("user cache directory is unavailable")
	}
	// QStandardPaths::CacheLocation for application name Harbor.
	return filepath.Clean(filepath.Join(cache, "Harbor", "cache", "harbor-updater")), nil
}

func samePath(a, b string) bool {
	return strings.EqualFold(filepath.Clean(a), filepath.Clean(b))
}
