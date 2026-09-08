package updater

import (
	"archive/tar"
	"archive/zip"
	"compress/gzip"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

const (
	MaxEntries    = 100000
	MaxFileBytes  = int64(1 << 30)
	MaxTotalBytes = int64(4 << 30)
	MaxRatio      = int64(100)
)

type Request struct {
	Package, Manifest, Install, Version, Transaction, UpdaterRoot, TransactionDir string
	Marker, Result, Log, Secret                                                   string // test-only overrides; CLI derives fixed names.
	PublicKeys                                                                    string // test-only override; production uses UpdatePublicKeys.
	LaunchToken                                                                   uintptr
	Parent                                                                        int
	ParentTimeout, HealthTimeout                                                  time.Duration
	Format                                                                        string
}
type Failure struct{ Stage, Code, Detail string }
type Result struct {
	Updated bool
	Failure *Failure
}

// Package-private operation seams keep destructive failure paths testable.
// Production always uses these defaults; the helper CLI cannot replace them.
var (
	renamePath        = os.Rename
	removeTree        = os.RemoveAll
	launchApplication = launch
	replaceFile       = atomicReplace
	syncDir           = syncDirectory
)

func fail(r Request, stage, code, detail string) Result {
	logf(r, stage+" "+code+": "+detail)
	f := Failure{stage, code, detail}
	state := "FAILED"
	if stage == "rollback" || code == "backup_cleanup" || code == "staging_cleanup" || code == "result_write" {
		state = "RECOVERY_REQUIRED"
	}
	persisted := writeJSON(r.Result, map[string]any{"result": state, "transaction": r.Transaction, "version": r.Version, "stage": stage, "error": code})
	if state == "RECOVERY_REQUIRED" && !persisted {
		code = "recovery_persistence_failed"
		detail += "; recovery state persistence failed"
	}
	f = Failure{Stage: stage, Code: code, Detail: detail}
	return Result{Failure: &f}
}
func recovery(r Request, code, detail string, paths map[string]string) Result {
	value := map[string]any{"result": "RECOVERY_REQUIRED", "transaction": r.Transaction, "version": r.Version, "error": code, "detail": detail, "paths": paths}
	// The sibling transaction directory has a deterministic name and survives
	// application-tree replacement. Persist there first so recovery remains
	// discoverable even if the cache result path becomes unavailable.
	recoveryDir := filepath.Join(filepath.Dir(r.Install), ".harbor-tx-"+r.Transaction)
	recoveryPath := filepath.Join(recoveryDir, "state.json")
	statePersisted := true
	if info, err := os.Lstat(recoveryDir); os.IsNotExist(err) {
		statePersisted = os.Mkdir(recoveryDir, 0700) == nil && syncDir(filepath.Dir(recoveryDir)) == nil
	} else if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		statePersisted = false
	}
	statePersisted = statePersisted && writeJSON(recoveryPath, map[string]any{
		"transaction_state": "RECOVERY_REQUIRED",
		"transaction":       r.Transaction,
		"version":           r.Version,
		"error":             code,
		"detail":            detail,
		"paths":             paths,
	})
	resultPersisted := writeJSON(r.Result, value)
	if !statePersisted || !resultPersisted {
		code = "recovery_persistence_failed"
		detail += "; durable recovery record incomplete; preserve transaction trees"
	}
	logf(r, "rollback recovery required: "+code+": "+detail)
	f := Failure{Stage: "rollback", Code: code, Detail: detail}
	return Result{Failure: &f}
}

func rolledBack(r Request, code, detail, txdir, failedTree string) Result {
	paths := map[string]string{"install": r.Install, "transaction_tree": txdir, "failed": failedTree}
	if failedTree != "" {
		if _, err := os.Lstat(failedTree); err == nil {
			if err = removeTree(failedTree); err != nil {
				return recovery(r, "failed_tree_cleanup", err.Error(), paths)
			}
		} else if !os.IsNotExist(err) {
			return recovery(r, "failed_tree_inspect", err.Error(), paths)
		}
	}
	if txdir != "" {
		if err := removeTree(txdir); err != nil {
			return recovery(r, "transaction_cleanup", err.Error(), paths)
		}
	}
	return fail(r, "health", code, detail+"; previous version restored")
}
func writeTransactionState(path, phase string, paths map[string]string) bool {
	return writeJSON(path, map[string]any{"transaction_state": phase, "paths": paths})
}
func startupConfirmed(p *os.Process) bool {
	done := make(chan struct{}, 1)
	go func() { _, _ = p.Wait(); done <- struct{}{} }()
	select {
	case <-done:
		return false
	case <-time.After(300 * time.Millisecond):
		return true
	}
}

func writableDirectory(path string) bool {
	probe := filepath.Join(path, ".harbor-update-write-probe-"+randomID())
	f, err := os.OpenFile(probe, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return false
	}
	closeErr := f.Close()
	removeErr := os.Remove(probe)
	return closeErr == nil && removeErr == nil
}
func logf(r Request, s string) {
	line := "[Updater] " + s + "\n"
	_, _ = os.Stderr.WriteString(line)
	if r.Log != "" {
		f, e := os.OpenFile(r.Log, os.O_APPEND|os.O_WRONLY|os.O_CREATE, 0600)
		if e == nil {
			_, _ = f.WriteString(line)
			_ = f.Close()
		}
	}
}
func writeJSON(path string, v any) bool {
	if path == "" {
		return false
	}
	b, e := json.Marshal(v)
	if e != nil {
		return false
	}
	t := path + ".tmp-" + randomID()
	if e = os.WriteFile(t, b, 0600); e != nil {
		return false
	}
	f, e := os.OpenFile(t, os.O_WRONLY, 0600)
	if e != nil {
		_ = os.Remove(t)
		return false
	}
	if e = f.Sync(); e == nil {
		e = f.Close()
	} else {
		_ = f.Close()
	}
	if e != nil || replaceFile(t, path) != nil {
		_ = os.Remove(t)
		return false
	}
	return syncReplacedFileDirectory(filepath.Dir(path)) == nil
}
func randomID() string {
	b := make([]byte, 16)
	if _, e := rand.Read(b); e != nil {
		return ""
	}
	return hex.EncodeToString(b)
}
func ValidTransaction(s string) bool {
	if len(s) != 36 {
		return false
	}
	for i := 0; i < len(s); i++ {
		separator := i == 8 || i == 13 || i == 18 || i == 23
		if separator {
			if s[i] != '-' {
				return false
			}
			continue
		}
		if s[i] == '-' || !strings.ContainsRune("0123456789abcdef", rune(s[i])) {
			return false
		}
	}
	return s[14] == '4' && strings.ContainsRune("89ab", rune(s[19]))
}
func NewTransaction() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return ""
	}
	b[6] = (b[6] & 0x0f) | 0x40
	b[8] = (b[8] & 0x3f) | 0x80
	h := hex.EncodeToString(b)
	return h[0:8] + "-" + h[8:12] + "-" + h[12:16] + "-" + h[16:20] + "-" + h[20:32]
}

func safeName(name string) bool {
	if name == "" || strings.Contains(name, "\\") || filepath.IsAbs(name) {
		return false
	}
	parts := strings.Split(strings.ReplaceAll(name, "/", "/"), "/")
	for _, part := range parts {
		if part == "" || part == "." || part == ".." {
			return false
		}
	}
	return true
}
func validateTransaction(root, txdir, tx string) error {
	expectedRoot, err := canonicalUpdaterRoot()
	if err != nil {
		return err
	}
	for _, p := range []string{root, txdir} {
		if e := validatePath(p); e != nil {
			return e
		}
	}
	rc, tc := filepath.Clean(root), filepath.Clean(txdir)
	if !samePath(rc, expectedRoot) {
		return errors.New("updater root is not the canonical user cache root")
	}
	ri, e := os.Lstat(rc)
	if e != nil || !ri.IsDir() || ri.Mode()&os.ModeSymlink != 0 || !securePrivatePath(rc) {
		return errors.New("private updater root required")
	}
	ti, e := os.Lstat(tc)
	if e != nil || !ti.IsDir() || ti.Mode()&os.ModeSymlink != 0 || !securePrivatePath(tc) {
		return errors.New("private transaction directory required")
	}
	if filepath.Dir(tc) != rc || filepath.Base(tc) != tx {
		return errors.New("transaction must be an immediate root child")
	}
	canonicalRoot, e := filepath.EvalSymlinks(rc)
	if e != nil {
		return e
	}
	canonicalTx, e := filepath.EvalSymlinks(tc)
	if e != nil {
		return e
	}
	if canonicalRoot != rc || canonicalTx != tc {
		return errors.New("canonical transaction paths required")
	}
	return nil
}
func validatePath(p string) error {
	if p == "" || !filepath.IsAbs(p) {
		return errors.New("absolute path required")
	}
	clean := filepath.Clean(p)
	for current := clean; ; current = filepath.Dir(current) {
		if info, err := os.Lstat(current); err == nil && info.Mode()&os.ModeSymlink != 0 {
			return errors.New("symlink path component")
		}
		parent := filepath.Dir(current)
		if parent == current {
			break
		}
	}
	return nil
}
func regularNoLink(p string) bool {
	i, e := os.Lstat(p)
	return e == nil && i.Mode().IsRegular() && i.Mode()&os.ModeSymlink == 0
}
func privatePath(p string) bool {
	i, e := os.Lstat(p)
	return e == nil && i.Mode().IsRegular() && securePrivatePath(p) && i.Mode()&os.ModeSymlink == 0
}
func extract(r Request, stage string) (string, error) {
	seen := map[string]bool{}
	total := int64(0)
	roots := map[string]bool{}
	add := func(name string, size int64, mode os.FileMode) (string, error) {
		rawName := name
		if strings.HasSuffix(rawName, "/") {
			rawName = strings.TrimSuffix(rawName, "/")
			if strings.HasSuffix(rawName, "/") {
				return "", errors.New("unsafe archive path")
			}
		}
		if !safeName(rawName) {
			return "", errors.New("unsafe archive path")
		}
		n := filepath.ToSlash(rawName)
		parts := strings.Split(n, "/")
		roots[parts[0]] = true
		if r.Format != "zip" && len(roots) > 1 {
			return "", errors.New("multiple roots")
		}
		if seen[strings.ToLower(n)] {
			return "", errors.New("duplicate or case-colliding path")
		}
		seen[strings.ToLower(n)] = true
		if len(seen) > MaxEntries || size < 0 || size > MaxFileBytes || total+size > MaxTotalBytes {
			return "", errors.New("archive size limit")
		}
		total += size
		if mode&os.ModeSymlink != 0 || mode&os.ModeNamedPipe != 0 || mode&os.ModeDevice != 0 {
			return "", errors.New("special file")
		}
		out := filepath.Join(stage, n)
		if !strings.HasPrefix(filepath.Clean(out), filepath.Clean(stage)+string(os.PathSeparator)) {
			return "", errors.New("staging escape")
		}
		return out, nil
	}
	if r.Format == "zip" {
		z, e := zip.OpenReader(r.Package)
		if e != nil {
			return "", e
		}
		defer z.Close()
		for _, f := range z.File {
			if f.FileInfo().Mode()&os.ModeSymlink != 0 {
				return "", errors.New("symlink")
			}
			out, e := add(f.Name, int64(f.UncompressedSize64), f.FileInfo().Mode())
			if e != nil {
				return "", e
			}
			if strings.HasSuffix(f.Name, "/") {
				if e = os.MkdirAll(out, 0700); e != nil {
					return "", e
				}
				continue
			}
			in, e := f.Open()
			if e != nil {
				return "", e
			}
			if int64(f.CompressedSize64) > 0 && int64(f.UncompressedSize64) > MaxRatio*int64(f.CompressedSize64) {
				_ = in.Close()
				return "", errors.New("expansion ratio limit")
			}
			e = writeEntry(out, in, int64(f.UncompressedSize64))
			_ = in.Close()
			if e != nil {
				return "", e
			}
		}
	} else {
		in, e := os.Open(r.Package)
		if e != nil {
			return "", e
		}
		defer in.Close()
		gz, e := gzip.NewReader(in)
		if e != nil {
			return "", e
		}
		defer gz.Close()
		tr := tar.NewReader(gz)
		for {
			h, e := tr.Next()
			if e == io.EOF {
				break
			}
			if e != nil {
				return "", e
			}
			if h.Typeflag != tar.TypeReg && h.Typeflag != tar.TypeDir {
				return "", errors.New("link or special file")
			}
			out, e := add(h.Name, h.Size, os.FileMode(h.Mode))
			if e != nil {
				return "", e
			}
			if h.Typeflag == tar.TypeDir {
				if e = os.MkdirAll(out, 0700); e != nil {
					return "", e
				}
				continue
			}
			if e = writeEntry(out, tr, h.Size); e != nil {
				return "", e
			}
		}
	}
	if len(roots) == 0 || (r.Format != "zip" && len(roots) != 1) {
		return "", errors.New("missing root")
	}
	if info, e := os.Stat(r.Package); e != nil || (info.Size() > 0 && total > info.Size()*MaxRatio) {
		return "", errors.New("expansion ratio limit")
	}
	root := stage
	if r.Format != "zip" {
		root = filepath.Join(stage, first(roots))
	}
	return root, nil
}
func writeEntry(out string, src io.Reader, size int64) error {
	if err := os.MkdirAll(filepath.Dir(out), 0700); err != nil {
		return err
	}
	f, err := os.OpenFile(out, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0700)
	if err != nil {
		return err
	}
	n, err := io.CopyN(f, src, size)
	if closeErr := f.Close(); err == nil {
		err = closeErr
	}
	if err != nil || n != size {
		return errors.New("short archive entry")
	}
	return nil
}
func first(m map[string]bool) string {
	for k := range m {
		return k
	}
	return ""
}

func markerValid(path, secret, tx, version string) bool {
	b, e := os.ReadFile(path)
	if e != nil {
		return false
	}
	var v map[string]any
	if json.Unmarshal(b, &v) != nil {
		return false
	}
	if v["transaction"] != tx || v["version"] != version {
		return false
	}
	sig, ok := v["hmac"].(string)
	if !ok {
		return false
	}
	key, e := os.ReadFile(secret)
	if e != nil {
		return false
	}
	mac := hmac.New(sha256.New, key)
	io.WriteString(mac, "harbor-health-v1\n"+version+"\n"+tx)
	return hmac.Equal([]byte(sig), []byte(hex.EncodeToString(mac.Sum(nil))))
}
func launch(program string, r Request) (*os.Process, error) {
	c := exec.Command(program)
	c.Dir = filepath.Dir(program)
	c.Env = append(os.Environ(), "HARBOR_HEALTH_TRANSACTION_DIR="+r.TransactionDir, "HARBOR_HEALTH_TRANSACTION="+r.Transaction, "HARBOR_HEALTH_EXPECTED_VERSION="+r.Version)
	configureLaunch(c, r.LaunchToken)
	if e := c.Start(); e != nil {
		return nil, e
	}
	return c.Process, nil
}

func Apply(r Request) Result {
	if !ValidTransaction(r.Transaction) {
		return fail(r, "validate", "invalid_transaction", "transaction must be generated UUID")
	}
	if r.UpdaterRoot != "" {
		r.Result = filepath.Join(r.UpdaterRoot, r.Transaction+".result.json")
	}
	keys := UpdatePublicKeys
	if r.PublicKeys != "" {
		keys = r.PublicKeys
	}
	if keys != "" && r.Manifest == "" {
		return fail(r, "validate", "signed_manifest_required", "release build requires a signed update manifest")
	}
	if r.Manifest != "" {
		platform := "linux"
		if r.Format == "zip" {
			platform = "windows"
		}
		if _, e := VerifySignedManifest(r.Manifest, r.Package, r.Version, platform, keys); e != nil {
			return fail(r, "validate", "invalid_signed_manifest", e.Error())
		}
	}
	if e := validateTransaction(r.UpdaterRoot, r.TransactionDir, r.Transaction); e != nil {
		return fail(r, "validate", "invalid_transaction_dir", e.Error())
	}
	if r.Parent <= 0 {
		return fail(r, "validate", "invalid_parent_pid", "parent PID must be positive")
	}
	waited, waitErr := waitPID(r.Parent, r.ParentTimeout)
	if waitErr != nil {
		return fail(r, "wait", "parent_wait_error", waitErr.Error())
	}
	if !waited {
		return fail(r, "wait", "parent_timeout", "parent did not exit")
	}
	for _, p := range []string{r.Package, r.Install} {
		if e := validatePath(p); e != nil {
			return fail(r, "validate", "invalid_path", e.Error())
		}
	}
	r.Marker = filepath.Join(r.TransactionDir, "health.json")
	// Result lives beside the transaction directory so successful cleanup does
	// not erase the durable outcome record.
	r.Result = filepath.Join(r.UpdaterRoot, r.Transaction+".result.json")
	r.Log = filepath.Join(r.TransactionDir, "updater.log")
	r.Secret = filepath.Join(r.TransactionDir, "health.secret")
	for _, p := range []string{r.Marker, r.Result, r.Log, r.Secret} {
		if e := validatePath(p); e != nil {
			return fail(r, "validate", "invalid_transaction_dir", e.Error())
		}
	}
	if r.Format != "zip" && r.Format != "targz" {
		return fail(r, "validate", "invalid_format", r.Format)
	}
	if !regularNoLink(r.Package) {
		return fail(r, "validate", "invalid_package", "package is not a regular file")
	}
	if !privatePath(r.Secret) {
		return fail(r, "validate", "insecure_secret", "secret file must be private")
	}
	parent := filepath.Dir(r.Install)
	parentInfo, parentErr := os.Stat(parent)
	if parentErr != nil || !parentInfo.IsDir() || !writableDirectory(parent) {
		return fail(r, "validate", "unwritable_parent", "install parent is not writable")
	}
	stale, _ := filepath.Glob(filepath.Join(parent, ".harbor-backup-*"))
	if _, e := os.Lstat(r.Install); os.IsNotExist(e) && len(stale) == 1 {
		if e = renamePath(stale[0], r.Install); e != nil {
			return recovery(r, "interrupted_recovery", e.Error(), map[string]string{"install": r.Install, "backup": stale[0]})
		}
	} else if len(stale) > 0 {
		return fail(r, "validate", "interrupted_transaction", "existing backup requires explicit recovery")
	}
	ii, e := os.Lstat(r.Install)
	if e != nil || !ii.IsDir() || ii.Mode()&os.ModeSymlink != 0 || filepath.Clean(r.Install) == filepath.Dir(filepath.VolumeName(r.Install)+string(os.PathSeparator)) {
		return fail(r, "validate", "unsafe_install", "install directory rejected")
	}
	if strings.HasPrefix(filepath.Clean(r.Package), filepath.Clean(r.Install)+string(os.PathSeparator)) {
		return fail(r, "validate", "package_inside_install", "package is inside install")
	}
	if backupExists, _ := os.Lstat(filepath.Join(parent, ".harbor-backup-"+r.Transaction)); backupExists != nil {
		return fail(r, "validate", "transaction_exists", "transaction backup already exists")
	}
	txdir := filepath.Join(parent, ".harbor-tx-"+r.Transaction)
	stage := filepath.Join(txdir, "stage")
	backup := filepath.Join(parent, ".harbor-backup-"+r.Transaction)
	failedTree := filepath.Join(parent, ".harbor-failed-"+r.Transaction)
	statePath := filepath.Join(txdir, "state.json")
	if info, statErr := os.Lstat(txdir); statErr == nil {
		if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return fail(r, "validate", "interrupted_transaction", "transaction state path is unsafe")
		}
		return fail(r, "validate", "interrupted_transaction", "transaction state already exists")
	}
	if e = os.MkdirAll(stage, 0700); e != nil {
		return fail(r, "prepare", "stage_create", e.Error())
	}
	if !writeTransactionState(statePath, "PREPARED", map[string]string{"install": r.Install, "backup": backup, "staged": stage, "failed": failedTree}) {
		return fail(r, "prepare", "state_write", "transaction state could not be durably written")
	}
	root, e := extract(r, stage)
	if e != nil {
		return fail(r, "prepare", "archive_rejected", e.Error())
	}
	if r.Format == "targz" && filepath.Base(root) != "harbor-linux-x86_64" {
		return fail(r, "prepare", "archive_rejected", "Linux archive must have harbor-linux-x86_64 root")
	}
	if string(mustRead(filepath.Join(root, "version.txt"))) != r.Version+"\n" && strings.TrimSpace(string(mustRead(filepath.Join(root, "version.txt")))) != r.Version {
		return fail(r, "prepare", "wrong_version", "version mismatch")
	}
	ext := ""
	if r.Format == "zip" {
		ext = ".exe"
	}
	for _, n := range []string{"harbor", "harbor-core", "harbor-media", "harbor-update-helper"} {
		if !regularNoLink(filepath.Join(root, n+ext)) {
			return fail(r, "prepare", "missing_executable", n)
		}
	}
	if e = renamePath(r.Install, backup); e != nil {
		return fail(r, "apply", "rename_old", e.Error())
	}
	if !writeTransactionState(statePath, "OLD_TREE_MOVED", map[string]string{"install": r.Install, "backup": backup, "staged": root, "failed": failedTree}) {
		return recovery(r, "state_write", "transaction state could not be updated", map[string]string{"install": r.Install, "backup": backup, "staged": root, "failed": failedTree})
	}
	if e = renamePath(root, r.Install); e != nil {
		if restoreErr := renamePath(backup, r.Install); restoreErr != nil {
			return recovery(r, "restore_failed", restoreErr.Error(), map[string]string{"install": r.Install, "backup": backup, "staged": root})
		}
		if old, oldErr := launchApplication(filepath.Join(r.Install, "harbor"+ext), r); oldErr != nil {
			return recovery(r, "old_relaunch_failed", oldErr.Error(), map[string]string{"install": r.Install, "staged": root})
		} else if !startupConfirmed(old) {
			return recovery(r, "old_relaunch_failed", "old Harbor exited during startup", map[string]string{"install": r.Install, "staged": root})
		}
		return rolledBack(r, "rename_stage_rolled_back", e.Error(), txdir, "")
	}
	if !writeTransactionState(statePath, "NEW_TREE_INSTALLED", map[string]string{"install": r.Install, "backup": backup, "staged": root, "failed": failedTree}) {
		return recovery(r, "state_write", "transaction state could not be updated", map[string]string{"install": r.Install, "backup": backup, "staged": root, "failed": failedTree})
	}
	p, e := launchApplication(filepath.Join(r.Install, "harbor"+ext), r)
	if e != nil {
		if moveErr := renamePath(r.Install, failedTree); moveErr != nil {
			return recovery(r, "failed_tree_move", moveErr.Error(), map[string]string{"install": r.Install, "backup": backup, "failed": failedTree})
		}
		if re := renamePath(backup, r.Install); re != nil {
			return recovery(r, "restore_failed", re.Error(), map[string]string{"install": r.Install, "backup": backup, "failed": failedTree})
		}
		if old, oldErr := launchApplication(filepath.Join(r.Install, "harbor"+ext), r); oldErr != nil {
			return recovery(r, "old_relaunch_failed", oldErr.Error(), map[string]string{"install": r.Install, "failed": failedTree})
		} else if !startupConfirmed(old) {
			return recovery(r, "old_relaunch_failed", "old Harbor exited during startup", map[string]string{"install": r.Install, "failed": failedTree})
		}
		return rolledBack(r, "launch_failed_rolled_back", e.Error(), txdir, failedTree)
	}
	childDone := make(chan error, 1)
	go func() { _, waitError := p.Wait(); childDone <- waitError }()
	deadline := time.Now().Add(r.HealthTimeout)
	healthy := false
	childExited := false
	for time.Now().Before(deadline) {
		if markerValid(r.Marker, r.Secret, r.Transaction, r.Version) {
			healthy = true
			break
		}
		select {
		case <-childDone:
			childExited = true
		case <-time.After(25 * time.Millisecond):
		}
		if childExited {
			break
		}
	}
	if !healthy {
		if !childExited {
			if e = gracefulTerminate(p); e != nil {
				return recovery(r, "terminate_failed", e.Error(), map[string]string{"install": r.Install, "backup": backup})
			}
			select {
			case <-childDone:
			case <-time.After(3 * time.Second):
				if e = p.Kill(); e != nil {
					return recovery(r, "kill_failed", e.Error(), map[string]string{"install": r.Install, "backup": backup})
				}
				select {
				case <-childDone:
				case <-time.After(3 * time.Second):
					return recovery(r, "terminate_timeout", "child did not exit", map[string]string{"install": r.Install, "backup": backup})
				}
			}
		}
		if e = renamePath(r.Install, failedTree); e != nil {
			return recovery(r, "failed_tree_move", e.Error(), map[string]string{"install": r.Install, "backup": backup, "failed": failedTree})
		}
		if e = renamePath(backup, r.Install); e != nil {
			return recovery(r, "restore_failed", e.Error(), map[string]string{"install": r.Install, "backup": backup, "failed": failedTree})
		}
		if old, oldErr := launchApplication(filepath.Join(r.Install, "harbor"+ext), r); oldErr != nil {
			return recovery(r, "old_relaunch_failed", oldErr.Error(), map[string]string{"install": r.Install, "failed": failedTree})
		} else if !startupConfirmed(old) {
			return recovery(r, "old_relaunch_failed", "old Harbor exited during startup", map[string]string{"install": r.Install, "failed": failedTree})
		}
		return rolledBack(r, "health_failed_rolled_back", "health handshake failed", txdir, failedTree)
	}
	if !writeTransactionState(statePath, "HEALTHY", map[string]string{"install": r.Install, "backup": backup, "staged": root, "failed": failedTree}) {
		return recovery(r, "state_write", "transaction state could not be updated", map[string]string{"install": r.Install, "backup": backup, "staged": root, "failed": failedTree})
	}
	if e = removeTree(backup); e != nil {
		return recovery(r, "backup_cleanup", e.Error(), map[string]string{"install": r.Install, "backup": backup, "transaction_tree": txdir})
	}
	if e = removeTree(txdir); e != nil {
		return recovery(r, "staging_cleanup", e.Error(), map[string]string{"install": r.Install, "transaction_tree": txdir})
	}
	if !writeJSON(r.Result, map[string]any{"result": "UPDATED", "transaction": r.Transaction, "version": r.Version}) {
		return recovery(r, "result_write", "updated tree is healthy but result persistence failed", map[string]string{"install": r.Install})
	}
	return Result{Updated: true}
}
func mustRead(p string) []byte { b, _ := os.ReadFile(p); return b }
