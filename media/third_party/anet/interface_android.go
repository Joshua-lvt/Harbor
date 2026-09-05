//go:build android

package anet

import "net"

// Android's public net package is sufficient for Pion's candidate gathering.
// Do not link into net's private zone cache: that breaks as Go internals
// change and is unnecessary for Harbor's host-candidate policy.
func Interfaces() ([]net.Interface, error) { return net.Interfaces() }
func InterfaceAddrs() ([]net.Addr, error) { return net.InterfaceAddrs() }
func InterfaceAddrsByInterface(ifi *net.Interface) ([]net.Addr, error) { return ifi.Addrs() }
func SetAndroidVersion(version uint) {}
