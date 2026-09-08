//go:build !windows

package updater

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestHelperCLIReplacesInstallAndConfirmsHealth(t *testing.T) {
	r, install, resultPath := applyFixture(t, "health")
	helper := filepath.Join(t.TempDir(), "harbor-update-helper")
	build := exec.Command("go", "build", "-o", helper, "./cmd/harbor-update-helper")
	if output, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build helper: %v\n%s", err, output)
	}

	parent := exec.Command("sleep", "30")
	if err := parent.Start(); err != nil {
		t.Fatal("start controlled parent:", err)
	}
	t.Cleanup(func() {
		_ = parent.Process.Kill()
		_ = parent.Wait()
	})

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, helper,
		"--package", r.Package,
		"--install-dir", r.Install,
		"--expected-version", r.Version,
		"--parent-pid", strconv.Itoa(parent.Process.Pid),
		"--updater-root", r.UpdaterRoot,
		"--transaction-dir", r.TransactionDir,
		"--transaction", r.Transaction,
		"--format", r.Format,
	)
	cmd.Env = os.Environ()
	var output bytes.Buffer
	cmd.Stdout = &output
	cmd.Stderr = &output
	if err := cmd.Start(); err != nil {
		t.Fatal("start helper:", err)
	}
	// The helper must observe the live parent before the controlled exit.
	time.Sleep(100 * time.Millisecond)
	if err := parent.Process.Kill(); err != nil {
		t.Fatal("stop controlled parent:", err)
	}
	if err := parent.Wait(); err == nil {
		t.Fatal("controlled parent unexpectedly exited successfully")
	}
	if err := cmd.Wait(); err != nil {
		if ctx.Err() != nil {
			t.Fatalf("helper timed out: %v\n%s", ctx.Err(), output.Bytes())
		}
		t.Fatalf("helper failed: %v\n%s", err, output.Bytes())
	}

	if _, err := os.Stat(filepath.Join(install, "new-only.txt")); err != nil {
		t.Fatal("new tree not installed:", err)
	}
	if _, err := os.Stat(filepath.Join(install, "old-only.txt")); !os.IsNotExist(err) {
		t.Fatal("old tree survived replacement")
	}
	body, err := os.ReadFile(resultPath)
	if err != nil || !strings.Contains(string(body), `"result":"UPDATED"`) {
		t.Fatalf("durable success missing: %v %s", err, body)
	}
	waitForChildDone(t, resultPath)
	assertTransactionTreesCleaned(t, r, install)
}
