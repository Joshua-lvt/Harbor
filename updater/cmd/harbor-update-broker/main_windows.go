package main

import (
	"fmt"
	"harbor/updater"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"
)

func main() {
	allowed := map[string]bool{"--package": true, "--manifest": true, "--expected-version": true, "--parent-pid": true, "--updater-root": true, "--transaction-dir": true, "--transaction": true, "--ready-event": true}
	values := map[string]string{}
	for i := 1; i < len(os.Args); i++ {
		key := os.Args[i]
		if !allowed[key] || values[key] != "" || i+1 >= len(os.Args) {
			fatal("invalid arguments", 2)
		}
		i++
		values[key] = os.Args[i]
	}
	for key := range allowed {
		if values[key] == "" {
			fatal("missing argument "+key, 2)
		}
	}
	if !updater.IsElevated() || updater.UpdatePublicKeys == "" {
		fatal("protected update broker is not trusted", 1)
	}
	pid, err := strconv.Atoi(values["--parent-pid"])
	if err != nil || pid <= 0 || !updater.ValidTransaction(values["--transaction"]) {
		fatal("invalid update identity", 2)
	}
	install, err := updater.RegisteredInstallDir()
	if err != nil {
		fatal(err.Error(), 1)
	}
	parentExecutable, err := updater.ProcessExecutable(pid)
	if err != nil || !strings.EqualFold(filepath.Clean(parentExecutable), filepath.Join(install, "harbor.exe")) {
		fatal("update parent is not the registered Harbor executable", 1)
	}
	currentVersion, err := os.ReadFile(filepath.Join(install, "version.txt"))
	if err != nil {
		fatal("installed version is unavailable", 1)
	}
	order, err := updater.CompareManifestVersions(values["--expected-version"], strings.TrimSpace(string(currentVersion)))
	if err != nil || order <= 0 {
		fatal("signed update is not newer than the installed version", 1)
	}
	token, err := updater.CaptureLaunchToken(pid)
	if err != nil {
		fatal("could not preserve the unelevated launch identity", 1)
	}
	defer syscall.CloseHandle(syscall.Handle(token))

	protected := filepath.Join(filepath.Dir(install), ".harbor-package-"+values["--transaction"])
	if err = os.Mkdir(protected, 0700); err != nil {
		fatal("could not create protected package staging", 1)
	}
	defer os.RemoveAll(protected)
	packagePath, manifestPath, err := updater.SnapshotSignedPackage(
		values["--manifest"], values["--package"], protected,
		values["--expected-version"], "windows", updater.UpdatePublicKeys)
	if err != nil {
		fatal("signed package rejected: "+err.Error(), 1)
	}
	if err = updater.SignalReadyEvent(values["--ready-event"]); err != nil {
		fatal("could not acknowledge protected update readiness", 1)
	}
	request := updater.Request{
		Package: packagePath, Manifest: manifestPath, Install: install,
		Version: values["--expected-version"], Transaction: values["--transaction"],
		UpdaterRoot: values["--updater-root"], TransactionDir: values["--transaction-dir"],
		Parent: pid, ParentTimeout: 2 * time.Minute, HealthTimeout: 2 * time.Minute,
		Format: "zip", LaunchToken: token,
	}
	result := updater.Apply(request)
	if !result.Updated {
		if result.Failure != nil && (result.Failure.Stage == "validate" ||
			result.Failure.Stage == "wait" || result.Failure.Stage == "prepare" ||
			result.Failure.Stage == "apply" && result.Failure.Code == "rename_old") {
			_ = updater.RelaunchAfterParent(pid, filepath.Join(install, "harbor.exe"), token)
		}
		os.Exit(1)
	}
}

func fatal(message string, code int) {
	fmt.Fprintln(os.Stderr, message)
	os.Exit(code)
}
