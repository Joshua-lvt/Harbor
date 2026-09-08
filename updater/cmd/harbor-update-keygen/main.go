package main

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"flag"
	"fmt"
	"os"
	"regexp"
)

var keyIDPattern = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]{0,31}$`)

func main() {
	keyID := flag.String("key-id", "", "public key identifier")
	privateOutput := flag.String("private-output", "", "new private seed output file")
	flag.Parse()
	if flag.NArg() != 0 || !keyIDPattern.MatchString(*keyID) || *privateOutput == "" {
		fmt.Fprintln(os.Stderr, "valid -key-id and -private-output are required")
		os.Exit(2)
	}
	public, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		fmt.Fprintln(os.Stderr, "could not generate Ed25519 key")
		os.Exit(1)
	}
	file, err := os.OpenFile(*privateOutput, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		fmt.Fprintln(os.Stderr, "private output must be a new file")
		os.Exit(1)
	}
	seed := base64.StdEncoding.EncodeToString(private.Seed()) + "\n"
	if _, err = file.WriteString(seed); err == nil {
		err = file.Sync()
	}
	if closeErr := file.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		_ = os.Remove(*privateOutput)
		fmt.Fprintln(os.Stderr, "could not persist private key")
		os.Exit(1)
	}
	fmt.Printf("HARBOR_UPDATE_KEY_ID=%s\nHARBOR_UPDATE_PUBLIC_KEYS=%s:%s\n",
		*keyID, *keyID, base64.StdEncoding.EncodeToString(public))
}
