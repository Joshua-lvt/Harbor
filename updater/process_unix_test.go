//go:build !windows

package updater

import (
	"os"
	"testing"
	"time"
)

func TestUnixCurrentProcessWaitIsBounded(t *testing.T) {
	start := time.Now()
	ended, err := waitPID(os.Getpid(), 20*time.Millisecond)
	if err != nil || ended {
		t.Fatalf("current process: ended=%v err=%v", ended, err)
	}
	if time.Since(start) > time.Second {
		t.Fatal("wait exceeded bound")
	}
}
