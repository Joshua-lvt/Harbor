//go:build !windows

package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "harbor-update-broker is Windows-only")
	os.Exit(2)
}
