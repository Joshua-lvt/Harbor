//go:build windows

package updater

import (
	"os"
	"syscall"
	"unsafe"
)

const (
	seFileObject                       = 1
	ownerSecurityInformation           = 1
	daclSecurityInformation            = 4
	tokenUser                          = 1
	aclSizeInformation                 = 2
	accessAllowedAceType               = 0
	accessAllowedCompoundAceType       = 4
	accessAllowedObjectAceType         = 5
	accessAllowedCallbackAceType       = 9
	accessAllowedCallbackObjectAceType = 11
	inheritOnlyAce                     = 0x08
	genericWrite                       = 0x40000000
	genericAll                         = 0x10000000
	fileWriteData                      = 2
	fileAppendData                     = 4
	fileWriteEA                        = 16
	fileWriteAttributes                = 256
	deleteAccess                       = 0x10000
	writeDAC                           = 0x40000
	writeOwner                         = 0x80000
)

var advapi = syscall.NewLazyDLL("advapi32.dll")
var kernel = syscall.NewLazyDLL("kernel32.dll")
var pGetNamed = advapi.NewProc("GetNamedSecurityInfoW")
var pGetOwner = advapi.NewProc("GetSecurityDescriptorOwner")
var pGetDacl = advapi.NewProc("GetSecurityDescriptorDacl")
var pEqual = advapi.NewProc("EqualSid")
var pConvert = advapi.NewProc("ConvertStringSidToSidW")
var pBuildTrustee = advapi.NewProc("BuildTrusteeWithSidW")
var pGetEffectiveRights = advapi.NewProc("GetEffectiveRightsFromAclW")
var pToken = advapi.NewProc("GetTokenInformation")
var pOpen = advapi.NewProc("OpenProcessToken")
var pFree = kernel.NewProc("LocalFree")
var pCurrent = kernel.NewProc("GetCurrentProcess")

func securePrivatePath(path string) bool {
	if i, e := os.Lstat(path); e != nil || !i.Mode().IsRegular() && !i.IsDir() || i.Mode()&os.ModeSymlink != 0 {
		return false
	}
	return secureWindowsPath(path)
}

// secureWindowsPath accepts only a path owned by the current user whose DACL
// grants no write access to the non-privileged local principals that could
// tamper with a transaction (Everyone, Authenticated Users, Users). SYSTEM,
// Administrators and CREATOR OWNER remain: they are privileged/marker SIDs,
// not a same-machine escalation vector. Rights are computed with
// GetEffectiveRightsFromAclW so deny ACEs and INHERIT_ONLY ACEs are honored
// exactly as the object manager would honor them.
func secureWindowsPath(path string) bool {
	p, _ := syscall.UTF16PtrFromString(path)
	var sd, owner, dacl unsafe.Pointer
	if r, _, _ := pGetNamed.Call(uintptr(unsafe.Pointer(p)), seFileObject, ownerSecurityInformation|daclSecurityInformation, uintptr(unsafe.Pointer(&owner)), 0, uintptr(unsafe.Pointer(&dacl)), 0, uintptr(unsafe.Pointer(&sd))); r != 0 {
		return false
	}
	defer pFree.Call(uintptr(sd))
	if sd == nil || owner == nil || dacl == nil {
		return false
	}
	if !sameCurrentSID(owner) {
		return false
	}
	const writable = genericWrite | genericAll | fileWriteData | fileAppendData | fileWriteEA | fileWriteAttributes | deleteAccess | writeDAC | writeOwner
	for _, threat := range []string{"S-1-1-0", "S-1-5-11", "S-1-5-32-545"} {
		if effectiveWriteShared(dacl, threat, writable) {
			return false
		}
	}
	return true
}

func effectiveWriteShared(dacl unsafe.Pointer, sidString string, writable uint32) bool {
	name, _ := syscall.UTF16PtrFromString(sidString)
	var sid unsafe.Pointer
	if r, _, _ := pConvert.Call(uintptr(unsafe.Pointer(name)), uintptr(unsafe.Pointer(&sid))); r == 0 {
		return false
	}
	defer pFree.Call(uintptr(sid))
	var trustee [64]byte
	pBuildTrustee.Call(uintptr(unsafe.Pointer(&trustee[0])), uintptr(sid))
	var rights uint32
	if r, _, _ := pGetEffectiveRights.Call(uintptr(dacl), uintptr(unsafe.Pointer(&trustee[0])), uintptr(unsafe.Pointer(&rights))); r != 0 {
		return false
	}
	return rights&writable != 0
}

func windowsPathOwnedByCurrentUser(path string) bool {
	p, err := syscall.UTF16PtrFromString(path)
	if err != nil {
		return false
	}
	var sd, owner unsafe.Pointer
	if r, _, _ := pGetNamed.Call(uintptr(unsafe.Pointer(p)), seFileObject, ownerSecurityInformation,
		uintptr(unsafe.Pointer(&owner)), 0, 0, 0, uintptr(unsafe.Pointer(&sd))); r != 0 {
		return false
	}
	defer pFree.Call(uintptr(sd))
	return sd != nil && owner != nil && sameCurrentSID(owner)
}
func sameCurrentSID(owner unsafe.Pointer) bool {
	var tok uintptr
	current, _, _ := pCurrent.Call()
	if r, _, _ := pOpen.Call(current, 0x0008|0x0002, uintptr(unsafe.Pointer(&tok))); r == 0 {
		return false
	}
	defer syscall.CloseHandle(syscall.Handle(tok))
	var n uint32
	pToken.Call(tok, tokenUser, 0, 0, uintptr(unsafe.Pointer(&n)))
	if n == 0 {
		return false
	}
	buf := make([]byte, n)
	if r, _, _ := pToken.Call(tok, tokenUser, uintptr(unsafe.Pointer(&buf[0])), uintptr(n), uintptr(unsafe.Pointer(&n))); r == 0 {
		return false
	}
	sid := *(*unsafe.Pointer)(unsafe.Pointer(&buf[0]))
	r, _, _ := pEqual.Call(uintptr(owner), uintptr(sid))
	return r != 0
}
