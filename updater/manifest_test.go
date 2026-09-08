package updater

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestSignedManifestVerificationAndTampering(t *testing.T) {
	dir := t.TempDir()
	packagePath := filepath.Join(dir, "harbor-windows-x86_64.zip")
	if err := os.WriteFile(packagePath, []byte("signed package"), 0600); err != nil {
		t.Fatal(err)
	}
	public, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := CreateSignedManifest(packagePath, "2.4.0", "windows", "release-2026", private)
	if err != nil {
		t.Fatal(err)
	}
	manifestPath := filepath.Join(dir, "update.json")
	body, _ := json.Marshal(manifest)
	if err = os.WriteFile(manifestPath, body, 0600); err != nil {
		t.Fatal(err)
	}
	keys := "release-2026:" + base64.StdEncoding.EncodeToString(public)
	if _, err = VerifySignedManifest(manifestPath, packagePath, "2.4.0", "windows", keys); err != nil {
		t.Fatal("valid signature rejected:", err)
	}

	if err = os.WriteFile(packagePath, []byte("tampered packag"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err = VerifySignedManifest(manifestPath, packagePath, "2.4.0", "windows", keys); err == nil {
		t.Fatal("tampered package accepted")
	}
	if err = os.WriteFile(packagePath, []byte("signed package"), 0600); err != nil {
		t.Fatal(err)
	}
	manifest.Version = "2.5.0"
	body, _ = json.Marshal(manifest)
	if err = os.WriteFile(manifestPath, body, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err = VerifySignedManifest(manifestPath, packagePath, "2.5.0", "windows", keys); err == nil {
		t.Fatal("tampered manifest accepted")
	}
}

func TestSignedManifestRejectsWrongKeyAndUnknownFields(t *testing.T) {
	dir := t.TempDir()
	packagePath := filepath.Join(dir, "harbor-linux-x86_64.tar.gz")
	if err := os.WriteFile(packagePath, []byte("archive"), 0600); err != nil {
		t.Fatal(err)
	}
	_, private, _ := ed25519.GenerateKey(rand.Reader)
	otherPublic, _, _ := ed25519.GenerateKey(rand.Reader)
	manifest, err := CreateSignedManifest(packagePath, "3.0.0", "linux", "primary", private)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := json.Marshal(manifest)
	manifestPath := filepath.Join(dir, "update.json")
	if err = os.WriteFile(manifestPath, body, 0600); err != nil {
		t.Fatal(err)
	}
	keys := "primary:" + base64.StdEncoding.EncodeToString(otherPublic)
	if _, err = VerifySignedManifest(manifestPath, packagePath, "3.0.0", "linux", keys); err == nil {
		t.Fatal("wrong key accepted")
	}
	body[len(body)-1] = ','
	body = append(body, []byte(`"extra":true}`)...)
	if err = os.WriteFile(manifestPath, body, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err = readSignedManifest(manifestPath); err == nil {
		t.Fatal("unknown manifest field accepted")
	}
}

func TestSignedManifestRejectsUnknownKeyID(t *testing.T) {
	dir := t.TempDir()
	packagePath := filepath.Join(dir, "harbor-windows-x86_64.zip")
	if err := os.WriteFile(packagePath, []byte("package"), 0600); err != nil {
		t.Fatal(err)
	}
	_, private, _ := ed25519.GenerateKey(rand.Reader)
	manifest, err := CreateSignedManifest(packagePath, "5.0.0", "windows", "unknown", private)
	if err != nil {
		t.Fatal(err)
	}
	manifestPath := filepath.Join(dir, "update.json")
	body, _ := json.Marshal(manifest)
	if err = os.WriteFile(manifestPath, body, 0600); err != nil {
		t.Fatal(err)
	}
	// A live key ring that does not contain "unknown" must reject, not panic.
	otherPublic, _, _ := ed25519.GenerateKey(rand.Reader)
	keys := "other:" + base64.StdEncoding.EncodeToString(otherPublic)
	if _, err = VerifySignedManifest(manifestPath, packagePath, "5.0.0", "windows", keys); err == nil {
		t.Fatal("unknown signing key was accepted")
	}
	snapshot := filepath.Join(dir, "protected")
	if err = os.Mkdir(snapshot, 0700); err != nil {
		t.Fatal(err)
	}
	if _, _, err = SnapshotSignedPackage(manifestPath, packagePath, snapshot, "5.0.0", "windows", keys); err == nil {
		t.Fatal("unknown signing key reached protected snapshot")
	}
}

func TestSnapshotSignedPackageCreatesProtectedCopy(t *testing.T) {
	dir := t.TempDir()
	packagePath := filepath.Join(dir, "harbor-windows-x86_64.zip")
	body := []byte("immutable signed package")
	if err := os.WriteFile(packagePath, body, 0600); err != nil {
		t.Fatal(err)
	}
	public, private, _ := ed25519.GenerateKey(rand.Reader)
	manifest, err := CreateSignedManifest(packagePath, "4.0.0", "windows", "primary", private)
	if err != nil {
		t.Fatal(err)
	}
	manifestBody, _ := json.Marshal(manifest)
	manifestPath := filepath.Join(dir, "source.update.json")
	if err = os.WriteFile(manifestPath, manifestBody, 0600); err != nil {
		t.Fatal(err)
	}
	destination := filepath.Join(dir, "protected")
	if err = os.Mkdir(destination, 0700); err != nil {
		t.Fatal(err)
	}
	keys := "primary:" + base64.StdEncoding.EncodeToString(public)
	snapshot, protectedManifest, err := SnapshotSignedPackage(
		manifestPath, packagePath, destination, "4.0.0", "windows", keys)
	if err != nil {
		t.Fatal(err)
	}
	if copied, err := os.ReadFile(snapshot); err != nil || string(copied) != string(body) {
		t.Fatalf("snapshot mismatch: %v %q", err, copied)
	}
	if _, err = VerifySignedManifest(protectedManifest, snapshot, "4.0.0", "windows", keys); err != nil {
		t.Fatal("protected snapshot failed verification:", err)
	}
}
