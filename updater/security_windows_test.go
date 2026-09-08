//go:build windows

package updater

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestWindowsPrivatePathEvaluator(t *testing.T) {
	d := hardenedTestDir(t)
	if !securePrivatePath(d) {
		t.Fatal("private temporary directory rejected")
	}
	if securePrivatePath(filepath.Join(d, "missing")) {
		t.Fatal("missing path accepted")
	}
	f := filepath.Join(d, "file")
	if err := os.WriteFile(f, []byte("x"), 0600); err != nil {
		t.Fatal(err)
	}
	hardenTestPath(t, f, false)
	if !securePrivatePath(f) {
		t.Fatal("private file rejected")
	}
}

func TestWindowsPrivatePathRejectsBroadWriteACL(t *testing.T) {
	d := filepath.Join(hardenedTestDir(t), "shared")
	if err := os.Mkdir(d, 0700); err != nil {
		t.Fatal(err)
	}
	hardenTestPath(t, d, true)
	// The SID form avoids localization of the built-in Everyone principal.
	command := exec.Command("icacls.exe", d, "/grant", "*S-1-1-0:(OI)(CI)M")
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("icacls grant failed: %v: %s", err, output)
	}
	if securePrivatePath(d) {
		t.Fatal("directory writable by Everyone was accepted as private")
	}
}
