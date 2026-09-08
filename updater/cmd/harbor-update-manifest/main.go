package main

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"flag"
	"fmt"
	"harbor/updater"
	"os"
)

func main() {
	packagePath := flag.String("package", "", "release package to sign")
	version := flag.String("version", "", "release semantic version")
	platform := flag.String("platform", "", "windows or linux")
	keyID := flag.String("key-id", "", "public key identifier")
	output := flag.String("output", "", "output manifest path")
	flag.Parse()
	if flag.NArg() != 0 || *packagePath == "" || *version == "" || *platform == "" || *keyID == "" || *output == "" {
		fmt.Fprintln(os.Stderr, "all flags are required")
		os.Exit(2)
	}
	encoded := os.Getenv("HARBOR_UPDATE_SIGNING_KEY")
	key, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		fmt.Fprintln(os.Stderr, "invalid HARBOR_UPDATE_SIGNING_KEY")
		os.Exit(2)
	}
	if len(key) == ed25519.SeedSize {
		key = ed25519.NewKeyFromSeed(key)
	}
	privateKey := ed25519.PrivateKey(key)
	if !updater.SigningKeyMatches(*keyID, privateKey, os.Getenv("HARBOR_UPDATE_PUBLIC_KEYS")) {
		fmt.Fprintln(os.Stderr, "signing key does not match HARBOR_UPDATE_PUBLIC_KEYS")
		os.Exit(2)
	}
	manifest, err := updater.CreateSignedManifest(*packagePath, *version, *platform, *keyID, privateKey)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	body, err := json.Marshal(manifest)
	if err != nil || os.WriteFile(*output, append(body, '\n'), 0600) != nil {
		fmt.Fprintln(os.Stderr, "could not write signed manifest")
		os.Exit(1)
	}
}
