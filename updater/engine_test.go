package updater

import (
	"archive/tar"
	"archive/zip"
	"compress/gzip"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestMain(m *testing.M) {
	if os.Getenv("HARBOR_UPDATER_TEST_CHILD") == "1" {
		runUpdaterTestChild()
		return
	}
	os.Exit(m.Run())
}

func runUpdaterTestChild() {
	executable, _ := os.Executable()
	mode, _ := os.ReadFile(filepath.Join(filepath.Dir(executable), "child-mode"))
	switch strings.TrimSpace(string(mode)) {
	case "health":
		txDir := os.Getenv("HARBOR_HEALTH_TRANSACTION_DIR")
		tx := os.Getenv("HARBOR_HEALTH_TRANSACTION")
		version := os.Getenv("HARBOR_HEALTH_EXPECTED_VERSION")
		key, err := os.ReadFile(filepath.Join(txDir, "health.secret"))
		if err != nil {
			os.Exit(21)
		}
		mac := hmac.New(sha256.New, key)
		_, _ = fmt.Fprint(mac, "harbor-health-v1\n", version, "\n", tx)
		body, _ := json.Marshal(map[string]any{
			"transaction": tx,
			"version":     version,
			"hmac":        hex.EncodeToString(mac.Sum(nil)),
			"ready":       true,
		})
		if err = os.WriteFile(filepath.Join(txDir, "health.json"), body, 0600); err != nil {
			os.Exit(22)
		}
		time.Sleep(500 * time.Millisecond)
		_ = os.WriteFile(os.Getenv("HARBOR_UPDATER_TEST_DONE"), []byte("health"), 0600)
	case "exit":
		os.Exit(23)
	case "invalid-health":
		txDir := os.Getenv("HARBOR_HEALTH_TRANSACTION_DIR")
		body, _ := json.Marshal(map[string]any{
			"transaction": os.Getenv("HARBOR_HEALTH_TRANSACTION"),
			"version":     os.Getenv("HARBOR_HEALTH_EXPECTED_VERSION"),
			"hmac":        strings.Repeat("0", 64),
			"ready":       true,
		})
		_ = os.WriteFile(filepath.Join(txDir, "health.json"), body, 0600)
		time.Sleep(500 * time.Millisecond)
		_ = os.WriteFile(os.Getenv("HARBOR_UPDATER_TEST_DONE"), []byte("invalid-health"), 0600)
	default:
		time.Sleep(500 * time.Millisecond)
		_ = os.WriteFile(os.Getenv("HARBOR_UPDATER_TEST_DONE"), []byte("old"), 0600)
	}
}

func makeTar(t *testing.T, path string, entries map[string]string) {
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	g := gzip.NewWriter(f)
	defer g.Close()
	w := tar.NewWriter(g)
	defer w.Close()
	for name, body := range entries {
		if err := w.WriteHeader(&tar.Header{Name: name, Mode: 0700, Size: int64(len(body))}); err != nil {
			t.Fatal(err)
		}
		if _, err := w.Write([]byte(body)); err != nil {
			t.Fatal(err)
		}
	}
}

func makeTarBytes(t *testing.T, path string, entries map[string][]byte) {
	t.Helper()
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	g := gzip.NewWriter(f)
	w := tar.NewWriter(g)
	for name, body := range entries {
		if err = w.WriteHeader(&tar.Header{Name: name, Mode: 0700, Size: int64(len(body))}); err != nil {
			t.Fatal(err)
		}
		if _, err = w.Write(body); err != nil {
			t.Fatal(err)
		}
	}
	if err = w.Close(); err != nil {
		t.Fatal(err)
	}
	if err = g.Close(); err != nil {
		t.Fatal(err)
	}
	if err = f.Close(); err != nil {
		t.Fatal(err)
	}
}
func makeZip(t *testing.T, path string, entries map[string]string) {
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	w := zip.NewWriter(f)
	for n, b := range entries {
		x, e := w.Create(n)
		if e != nil {
			t.Fatal(e)
		}
		_, _ = x.Write([]byte(b))
	}
	if e := w.Close(); e != nil {
		t.Fatal(e)
	}
	_ = f.Close()
}
func TestArchiveFormatsAndSpaces(t *testing.T) {
	d := t.TempDir()
	stage := filepath.Join(d, "stage with spaces")
	_ = os.Mkdir(stage, 0700)
	tarPath := filepath.Join(d, "release.tar.gz")
	makeTar(t, tarPath, map[string]string{"harbor-linux-x86_64/version.txt": "2.3.0\n", "harbor-linux-x86_64/harbor": "x"})
	root, e := extract(Request{Package: tarPath, Format: "targz"}, stage)
	if e != nil || filepath.Base(root) != "harbor-linux-x86_64" {
		t.Fatalf("tar: %v %s", e, root)
	}
	zipPath := filepath.Join(d, "release.zip")
	makeZip(t, zipPath, map[string]string{"version.txt": "2.3.0\n", "harbor.exe": "x"})
	if _, e = extract(Request{Package: zipPath, Format: "zip"}, filepath.Join(d, "zip stage")); e != nil {
		t.Fatal(e)
	}
}
func TestArchiveRejectsTraversalAndBackslash(t *testing.T) {
	d := t.TempDir()
	for i, name := range []string{"../escape", "C:\\escape"} {
		p := filepath.Join(d, "bad", string(rune('0'+i)))
		_ = os.MkdirAll(filepath.Dir(p), 0700)
		makeTar(t, p, map[string]string{name: "x"})
		s := filepath.Join(d, "stage", string(rune('0'+i)))
		_ = os.MkdirAll(s, 0700)
		if _, e := extract(Request{Package: p, Format: "targz"}, s); e == nil {
			t.Fatalf("accepted %q", name)
		}
	}
}
func TestTransactionAndMarkerContracts(t *testing.T) {
	if !ValidTransaction(NewTransaction()) {
		t.Fatal("generated transaction is not canonical UUID")
	}
	if !ValidTransaction("00000000-0000-4000-8000-000000000000") {
		t.Fatal("uuid rejected")
	}
	if ValidTransaction("not-a-transaction") {
		t.Fatal("invalid uuid accepted")
	}
	for _, value := range []string{"00000000-0000-4000-8000-00000000000A", "00000000-0000-5000-8000-000000000000", "00000000-0000-4000-7000-000000000000", "000000000-000-4000-8000-000000000000", "00000000--0000-4000-8000-000000000000", "0000000--0000-4000-8000-0000000000000", "00000000-0000-4000-8000-0000000000-0"} {
		if ValidTransaction(value) {
			t.Fatalf("accepted invalid UUID %q", value)
		}
	}
}
func TestSafeNameRejectsRawAmbiguity(t *testing.T) {
	for _, name := range []string{"", ".", "..", "a/../x", "a//x", "/absolute", `a\\b`} {
		if safeName(name) {
			t.Fatalf("accepted unsafe name %q", name)
		}
	}
}
func TestTransactionRootIsImmediatePrivateChild(t *testing.T) {
	d := hardenedTestDir(t)
	if runtime.GOOS == "windows" {
		t.Setenv("LOCALAPPDATA", d)
	} else {
		t.Setenv("XDG_CACHE_HOME", d)
	}
	root, err := canonicalUpdaterRoot()
	if err != nil {
		t.Fatal(err)
	}
	tx := filepath.Join(root, "00000000-0000-4000-8000-000000000000")
	if err := os.MkdirAll(tx, 0700); err != nil {
		t.Fatal(err)
	}
	hardenTestPath(t, root, true)
	hardenTestPath(t, tx, true)
	if err := validateTransaction(root, tx, filepath.Base(tx)); err != nil {
		t.Fatal(err)
	}
	if err := validateTransaction(root, filepath.Join(tx, "nested"), filepath.Base(tx)); err == nil {
		t.Fatal("accepted nested transaction")
	}
	link := filepath.Join(d, "link")
	if err := os.Symlink(root, link); err != nil {
		t.Fatal(err)
	}
	if err := validateTransaction(link, tx, filepath.Base(tx)); err == nil {
		t.Fatal("accepted symlink root")
	}
}

func applyFixture(t *testing.T, newMode string, replacementBinary ...[]byte) (Request, string, string) {
	t.Helper()
	base := hardenedTestDir(t)
	cache := filepath.Join(base, "cache")
	if runtime.GOOS == "windows" {
		t.Setenv("LOCALAPPDATA", cache)
	} else {
		t.Setenv("XDG_CACHE_HOME", cache)
	}
	root, err := canonicalUpdaterRoot()
	if err != nil {
		t.Fatal(err)
	}
	tx := NewTransaction()
	txDir := filepath.Join(root, tx)
	if err = os.MkdirAll(txDir, 0700); err != nil {
		t.Fatal(err)
	}
	hardenTestPath(t, root, true)
	hardenTestPath(t, txDir, true)
	secret := make([]byte, 32)
	if _, err = rand.Read(secret); err != nil {
		t.Fatal(err)
	}
	secretPath := filepath.Join(txDir, "health.secret")
	if err = os.WriteFile(secretPath, secret, 0600); err != nil {
		t.Fatal(err)
	}
	hardenTestPath(t, secretPath, false)
	install := filepath.Join(base, "install with spaces")
	if err = os.MkdirAll(install, 0700); err != nil {
		t.Fatal(err)
	}
	testExe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	binary, err := os.ReadFile(testExe)
	if err != nil {
		t.Fatal(err)
	}
	newBinary := binary
	if len(replacementBinary) == 1 {
		newBinary = replacementBinary[0]
	}
	ext := ""
	format := "targz"
	if runtime.GOOS == "windows" {
		ext = ".exe"
		format = "zip"
	}
	if err = os.WriteFile(filepath.Join(install, "harbor"+ext), binary, 0700); err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(install, "child-mode"), []byte("old"), 0600); err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(install, "old-only.txt"), []byte("preserve"), 0600); err != nil {
		t.Fatal(err)
	}
	entries := map[string][]byte{
		"version.txt":                []byte("9.9.9\n"),
		"harbor" + ext:               newBinary,
		"harbor-core" + ext:          []byte("core"),
		"harbor-media" + ext:         []byte("media"),
		"harbor-update-helper" + ext: []byte("helper"),
		"child-mode":                 []byte(newMode),
		"new-only.txt":               []byte("installed"),
	}
	packagePath := filepath.Join(base, "package")
	if format == "targz" {
		rooted := make(map[string][]byte, len(entries))
		for name, body := range entries {
			rooted["harbor-linux-x86_64/"+name] = body
		}
		makeTarBytes(t, packagePath, rooted)
	} else {
		stringEntries := make(map[string]string, len(entries))
		for name, body := range entries {
			stringEntries[name] = string(body)
		}
		makeZip(t, packagePath, stringEntries)
	}
	dead := exec.Command(testExe, "-test.run=^$")
	if err = dead.Start(); err != nil {
		t.Fatal(err)
	}
	pid := dead.Process.Pid
	if err = dead.Wait(); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HARBOR_UPDATER_TEST_CHILD", "1")
	resultPath := filepath.Join(root, tx+".result.json")
	t.Setenv("HARBOR_UPDATER_TEST_DONE", resultPath+".child.done")
	return Request{
		Package: packagePath, Install: install, Version: "9.9.9",
		Transaction: tx, UpdaterRoot: root, TransactionDir: txDir,
		Parent: pid, ParentTimeout: time.Second, HealthTimeout: time.Second,
		Format: format,
	}, install, resultPath
}

func TestApplyReplacesCompleteTreeAndPersistsSuccess(t *testing.T) {
	r, install, resultPath := applyFixture(t, "health")
	result := Apply(r)
	if !result.Updated || result.Failure != nil {
		t.Fatalf("apply failed: %+v", result.Failure)
	}
	if _, err := os.Stat(filepath.Join(install, "new-only.txt")); err != nil {
		t.Fatal("new tree not installed:", err)
	}
	if _, err := os.Stat(filepath.Join(install, "old-only.txt")); !os.IsNotExist(err) {
		t.Fatal("old partial tree survived replacement")
	}
	body, err := os.ReadFile(resultPath)
	if err != nil || !strings.Contains(string(body), `"result":"UPDATED"`) {
		t.Fatalf("durable success missing: %v %s", err, body)
	}
	waitForChildDone(t, resultPath)
	assertTransactionTreesCleaned(t, r, install)
}

func TestApplyRequiresValidSignedManifestBeforeMutation(t *testing.T) {
	r, install, resultPath := applyFixture(t, "health")
	asset := "harbor-linux-x86_64.tar.gz"
	if runtime.GOOS == "windows" {
		asset = "harbor-windows-x86_64.zip"
	}
	packagePath := filepath.Join(filepath.Dir(r.Package), asset)
	body, err := os.ReadFile(r.Package)
	if err != nil || os.WriteFile(packagePath, body, 0600) != nil {
		t.Fatal("prepare signed package:", err)
	}
	r.Package = packagePath
	public, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	platform := "linux"
	if runtime.GOOS == "windows" {
		platform = "windows"
	}
	manifest, err := CreateSignedManifest(r.Package, r.Version, platform, "test", private)
	if err != nil {
		t.Fatal(err)
	}
	r.Manifest = filepath.Join(filepath.Dir(r.Package), "signed.update.json")
	manifestBody, _ := json.Marshal(manifest)
	if err = os.WriteFile(r.Manifest, manifestBody, 0600); err != nil {
		t.Fatal(err)
	}
	r.PublicKeys = "test:" + base64.StdEncoding.EncodeToString(public)

	tampered := r
	tampered.Version = "9.9.10"
	result := Apply(tampered)
	if result.Failure == nil || result.Failure.Code != "invalid_signed_manifest" {
		t.Fatalf("tampered manifest was not rejected: %+v", result)
	}
	if _, err = os.Stat(filepath.Join(install, "old-only.txt")); err != nil {
		t.Fatal("signature failure modified install:", err)
	}

	result = Apply(r)
	if !result.Updated || result.Failure != nil {
		t.Fatalf("signed update failed: %+v", result.Failure)
	}
	waitForChildDone(t, resultPath)
}

func TestApplyRollsBackWhenNewHarborExits(t *testing.T) {
	r, install, resultPath := applyFixture(t, "exit")
	result := Apply(r)
	if result.Updated || result.Failure == nil || result.Failure.Code != "health_failed_rolled_back" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if body, err := os.ReadFile(filepath.Join(install, "old-only.txt")); err != nil || string(body) != "preserve" {
		t.Fatalf("old tree was not restored: %v %q", err, body)
	}
	if _, err := os.Stat(filepath.Join(install, "new-only.txt")); !os.IsNotExist(err) {
		t.Fatal("failed new tree remained installed")
	}
	body, err := os.ReadFile(resultPath)
	if err != nil || !strings.Contains(string(body), `"result":"FAILED"`) {
		t.Fatalf("rollback result missing: %v %s", err, body)
	}
	waitForChildDone(t, resultPath)
	assertTransactionTreesCleaned(t, r, install)
}

func TestApplyRollsBackWhenNewHarborCannotLaunch(t *testing.T) {
	r, install, resultPath := applyFixture(t, "health", []byte("not an executable"))
	result := Apply(r)
	if result.Updated || result.Failure == nil || result.Failure.Code != "launch_failed_rolled_back" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if body, err := os.ReadFile(filepath.Join(install, "old-only.txt")); err != nil || string(body) != "preserve" {
		t.Fatalf("old tree was not restored: %v %q", err, body)
	}
	body, err := os.ReadFile(resultPath)
	if err != nil || !strings.Contains(string(body), `"result":"FAILED"`) {
		t.Fatalf("rollback result missing: %v %s", err, body)
	}
	waitForChildDone(t, resultPath)
	assertTransactionTreesCleaned(t, r, install)
}

func TestApplyRejectsInvalidParentBeforeMutation(t *testing.T) {
	r, install, _ := applyFixture(t, "health")
	r.Parent = 0
	result := Apply(r)
	if result.Failure == nil || result.Failure.Code != "invalid_parent_pid" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if _, err := os.Stat(filepath.Join(install, "old-only.txt")); err != nil {
		t.Fatal("preflight failure modified the current install:", err)
	}
	backups, err := filepath.Glob(filepath.Join(filepath.Dir(install), ".harbor-backup-*"))
	if err != nil || len(backups) != 0 {
		t.Fatalf("preflight created a backup: %v %v", err, backups)
	}
}

func TestApplyRejectsInvalidHealthMarkerAndRollsBack(t *testing.T) {
	r, install, resultPath := applyFixture(t, "invalid-health")
	result := Apply(r)
	if result.Updated || result.Failure == nil || result.Failure.Code != "health_failed_rolled_back" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if _, err := os.Stat(filepath.Join(install, "old-only.txt")); err != nil {
		t.Fatal("old tree was not restored:", err)
	}
	waitForChildDone(t, resultPath)
	assertTransactionTreesCleaned(t, r, install)
}

func TestApplyPersistsRecoveryWhenFailedTreeCannotMove(t *testing.T) {
	r, install, _ := applyFixture(t, "exit")
	originalRename := renamePath
	renamePath = func(from, to string) error {
		if from == install && strings.Contains(filepath.Base(to), ".harbor-failed-") {
			return errors.New("injected failed-tree rename")
		}
		return os.Rename(from, to)
	}
	t.Cleanup(func() { renamePath = originalRename })
	result := Apply(r)
	if result.Failure == nil || result.Failure.Code != "failed_tree_move" {
		t.Fatalf("unexpected result: %+v", result)
	}
	state := filepath.Join(filepath.Dir(install), ".harbor-tx-"+r.Transaction, "state.json")
	body, err := os.ReadFile(state)
	if err != nil || !strings.Contains(string(body), "RECOVERY_REQUIRED") {
		t.Fatalf("durable recovery state missing: %v %s", err, body)
	}
	if _, err = os.Stat(filepath.Join(filepath.Dir(install), ".harbor-backup-"+r.Transaction)); err != nil {
		t.Fatal("known-good backup was not preserved:", err)
	}
}

func TestApplyPersistsRecoveryWhenBackupCannotRestore(t *testing.T) {
	r, install, _ := applyFixture(t, "exit")
	backup := filepath.Join(filepath.Dir(install), ".harbor-backup-"+r.Transaction)
	originalRename := renamePath
	renamePath = func(from, to string) error {
		if from == backup && to == install {
			return errors.New("injected restore failure")
		}
		return os.Rename(from, to)
	}
	t.Cleanup(func() { renamePath = originalRename })
	result := Apply(r)
	if result.Failure == nil || result.Failure.Code != "restore_failed" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if _, err := os.Stat(backup); err != nil {
		t.Fatal("backup was not preserved:", err)
	}
	state := filepath.Join(filepath.Dir(install), ".harbor-tx-"+r.Transaction, "state.json")
	if body, err := os.ReadFile(state); err != nil || !strings.Contains(string(body), "RECOVERY_REQUIRED") {
		t.Fatalf("durable recovery state missing: %v %s", err, body)
	}
}

func TestApplyPersistsRecoveryWhenRestoredHarborExitsImmediately(t *testing.T) {
	r, install, _ := applyFixture(t, "exit")
	if err := os.WriteFile(filepath.Join(install, "child-mode"), []byte("exit"), 0600); err != nil {
		t.Fatal(err)
	}
	result := Apply(r)
	if result.Failure == nil || result.Failure.Code != "old_relaunch_failed" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if _, err := os.Stat(filepath.Join(install, "old-only.txt")); err != nil {
		t.Fatal("old tree was not restored before relaunch failure:", err)
	}
	state := filepath.Join(filepath.Dir(install), ".harbor-tx-"+r.Transaction, "state.json")
	if body, err := os.ReadFile(state); err != nil || !strings.Contains(string(body), "RECOVERY_REQUIRED") {
		t.Fatalf("durable recovery state missing: %v %s", err, body)
	}
}

func TestApplyReportsRecoveryWhenUpdatedResultCannotPersist(t *testing.T) {
	r, install, resultPath := applyFixture(t, "health")
	originalReplace := replaceFile
	replaceFile = func(from, to string) error {
		if to == resultPath {
			return errors.New("injected result rename failure")
		}
		return atomicReplace(from, to)
	}
	t.Cleanup(func() { replaceFile = originalReplace })
	result := Apply(r)
	if result.Updated || result.Failure == nil || result.Failure.Code != "recovery_persistence_failed" {
		t.Fatalf("unexpected result: %+v", result)
	}
	if _, err := os.Stat(filepath.Join(install, "new-only.txt")); err != nil {
		t.Fatal("healthy updated tree should remain installed:", err)
	}
	state := filepath.Join(filepath.Dir(install), ".harbor-tx-"+r.Transaction, "state.json")
	if body, err := os.ReadFile(state); err != nil || !strings.Contains(string(body), "RECOVERY_REQUIRED") {
		t.Fatalf("fallback recovery state missing: %v %s", err, body)
	}
	waitForChildDone(t, resultPath)
}

func waitForChildDone(t *testing.T, resultPath string) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if _, err := os.Stat(resultPath + ".child.done"); err == nil {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("launched Harbor child did not finish")
}

func assertTransactionTreesCleaned(t *testing.T, r Request, install string) {
	t.Helper()
	parent := filepath.Dir(install)
	for _, path := range []string{
		filepath.Join(parent, ".harbor-tx-"+r.Transaction),
		filepath.Join(parent, ".harbor-failed-"+r.Transaction),
		filepath.Join(parent, ".harbor-backup-"+r.Transaction),
	} {
		if _, err := os.Lstat(path); !os.IsNotExist(err) {
			t.Fatalf("transaction artifact was not cleaned: %s (%v)", path, err)
		}
	}
}
