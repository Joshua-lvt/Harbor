//go:build !windows

package updater

import (
	"os"
	"path/filepath"
	"testing"
)

func TestApplyRefusesUnwritableParentBeforeRename(t *testing.T) {
	r, install, _ := applyFixture(t, "health")
	parent := filepath.Dir(install)
	if err := os.Chmod(parent, 0500); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(parent, 0700) })
	result := Apply(r)
	if result.Failure == nil || result.Failure.Code != "unwritable_parent" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if _, err := os.Stat(filepath.Join(install, "old-only.txt")); err != nil {
		t.Fatal("preflight failure modified the current install:", err)
	}
	backups, err := filepath.Glob(filepath.Join(parent, ".harbor-backup-*"))
	if err != nil || len(backups) != 0 {
		t.Fatalf("preflight created a backup: %v %v", err, backups)
	}
}
