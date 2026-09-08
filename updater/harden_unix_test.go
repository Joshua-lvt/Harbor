//go:build !windows

package updater

import "testing"

// On Unix the fixture permissions (0700/0600 + owner uid) are already the
// production contract, so there is nothing extra to harden.
func hardenTestPath(*testing.T, string, bool) {}

func hardenedTestDir(t *testing.T) string {
	t.Helper()
	return t.TempDir()
}
