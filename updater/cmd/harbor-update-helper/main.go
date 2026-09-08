package main

import (
	"fmt"
	"harbor/updater"
	"os"
	"strconv"
	"time"
)

func main() {
	allowed := map[string]bool{"--package": true, "--manifest": true, "--install-dir": true, "--expected-version": true, "--parent-pid": true, "--updater-root": true, "--transaction-dir": true, "--transaction": true, "--format": true}
	vals := map[string]string{}
	seen := map[string]bool{}
	for i := 1; i < len(os.Args); i++ {
		k := os.Args[i]
		if !allowed[k] || i+1 >= len(os.Args) || seen[k] {
			fmt.Fprintln(os.Stderr, "invalid arguments")
			os.Exit(2)
		}
		i++
		vals[k] = os.Args[i]
		seen[k] = true
	}
	for _, k := range []string{"--package", "--install-dir", "--expected-version", "--parent-pid", "--updater-root", "--transaction-dir", "--transaction", "--format"} {
		if vals[k] == "" {
			fmt.Fprintln(os.Stderr, "missing argument", k)
			os.Exit(2)
		}
	}
	pid, err := strconv.Atoi(vals["--parent-pid"])
	if err != nil || pid <= 0 {
		fmt.Fprintln(os.Stderr, "invalid parent PID")
		os.Exit(2)
	}
	r := updater.Request{Package: vals["--package"], Manifest: vals["--manifest"], Install: vals["--install-dir"], Version: vals["--expected-version"], UpdaterRoot: vals["--updater-root"], TransactionDir: vals["--transaction-dir"], Transaction: vals["--transaction"], Parent: pid, Format: vals["--format"], ParentTimeout: 2 * time.Minute, HealthTimeout: 2 * time.Minute}
	if !updater.ValidTransaction(r.Transaction) {
		fmt.Fprintln(os.Stderr, "invalid transaction")
		os.Exit(2)
	}
	if !updater.Apply(r).Updated {
		os.Exit(1)
	}
}
