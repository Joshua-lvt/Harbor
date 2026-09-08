//go:build windows

package updater

// Test-only production mirror: the real producer (C++ tryApplyNow) hardens
// the updater root, transaction directory, copied helper and health secret to
// a protected current-user-only DACL before launching. Test fixtures must do
// the same, because a bare t.TempDir() inherits the machine's broad default
// DACL and would fail validation exactly like an untampered runner should.
//
// Hardening installs a complete security descriptor with the exact current-
// user SID (os/user reports the token SID string on Windows, never a display
// name), removing inherited ACEs and granting full control to that SID only.

import (
	"os/user"
	"path/filepath"
	"syscall"
	"testing"
	"unsafe"
)

var pConvertStringSecurityDescriptor = advapi.NewProc("ConvertStringSecurityDescriptorToSecurityDescriptorW")
var pSetFileSecurity = advapi.NewProc("SetFileSecurityW")

func currentUserSIDString() (string, bool) {
	u, err := user.Current()
	if err != nil || u == nil || u.Uid == "" {
		return "", false
	}
	return u.Uid, true
}

func hardenedTestDir(t *testing.T) string {
	t.Helper()
	path, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	hardenTestPath(t, path, true)
	return path
}

func hardenTestPath(t *testing.T, path string, isDir bool) {
	t.Helper()
	sid, ok := currentUserSIDString()
	if !ok || sid == "" {
		t.Fatal("could not resolve current user SID for test hardening")
	}
	inheritance := ""
	if isDir {
		inheritance = "OICI"
	}
	// O: sets the owner, D:P protects the DACL from inheritance, and the sole
	// ACE grants the current user full control (inherited by children for dirs).
	sddl, err := syscall.UTF16PtrFromString("O:" + sid + "D:P(A;" + inheritance + ";GA;;;" + sid + ")")
	if err != nil {
		t.Fatal(err)
	}
	var descriptor unsafe.Pointer
	if r, _, callErr := pConvertStringSecurityDescriptor.Call(
		uintptr(unsafe.Pointer(sddl)), 1, uintptr(unsafe.Pointer(&descriptor)), 0,
	); r == 0 || descriptor == nil {
		t.Fatalf("create private security descriptor: %v", callErr)
	}
	defer pFree.Call(uintptr(descriptor))
	p, err := syscall.UTF16PtrFromString(path)
	if err != nil {
		t.Fatal(err)
	}
	const protectedDACL = 0x80000000
	if r, _, callErr := pSetFileSecurity.Call(
		uintptr(unsafe.Pointer(p)),
		ownerSecurityInformation|daclSecurityInformation|protectedDACL,
		uintptr(descriptor),
	); r == 0 {
		t.Fatalf("apply private security descriptor: %v", callErr)
	}
	if !securePrivatePath(path) {
		t.Fatal("hardened test path still not private:", path)
	}
}
