package updater

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"regexp"
	"strconv"
	"strings"
)

const ManifestSchema = "harbor-update-manifest-v1"

// UpdatePublicKeys is injected into release builds with -ldflags. The format
// is a comma-separated key ring of key-id:base64-ed25519-public-key entries.
var UpdatePublicKeys string

type SignedManifest struct {
	Schema       string `json:"schema"`
	KeyID        string `json:"key_id"`
	Version      string `json:"version"`
	Platform     string `json:"platform"`
	Architecture string `json:"architecture"`
	Asset        string `json:"asset"`
	Size         int64  `json:"size"`
	SHA256       string `json:"sha256"`
	Signature    string `json:"signature"`
}

var (
	manifestVersion = regexp.MustCompile(`^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$`)
	manifestKeyID   = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]{0,31}$`)
	manifestSHA256  = regexp.MustCompile(`^[0-9a-f]{64}$`)
)

func (m SignedManifest) canonicalPayload() ([]byte, error) {
	if m.Schema != ManifestSchema || !manifestKeyID.MatchString(m.KeyID) || !manifestVersion.MatchString(m.Version) {
		return nil, errors.New("invalid manifest identity")
	}
	if m.Platform != "windows" && m.Platform != "linux" {
		return nil, errors.New("invalid manifest platform")
	}
	if m.Architecture != "x86_64" {
		return nil, errors.New("invalid manifest architecture")
	}
	expectedAsset := "harbor-" + m.Platform + "-x86_64"
	if m.Platform == "windows" {
		expectedAsset += ".zip"
	} else {
		expectedAsset += ".tar.gz"
	}
	if m.Asset != expectedAsset || m.Size <= 0 || m.Size > MaxFileBytes*2 || !manifestSHA256.MatchString(m.SHA256) {
		return nil, errors.New("invalid manifest package")
	}
	return []byte(ManifestSchema + "\n" +
		"key_id=" + m.KeyID + "\n" +
		"version=" + m.Version + "\n" +
		"platform=" + m.Platform + "\n" +
		"architecture=" + m.Architecture + "\n" +
		"asset=" + m.Asset + "\n" +
		"size=" + strconv.FormatInt(m.Size, 10) + "\n" +
		"sha256=" + m.SHA256 + "\n"), nil
}

func parsePublicKeys(encoded string) (map[string]ed25519.PublicKey, error) {
	keys := make(map[string]ed25519.PublicKey)
	for _, entry := range strings.Split(encoded, ",") {
		parts := strings.SplitN(strings.TrimSpace(entry), ":", 2)
		if len(parts) != 2 || !manifestKeyID.MatchString(parts[0]) {
			return nil, errors.New("invalid update public key ring")
		}
		key, err := base64.StdEncoding.DecodeString(parts[1])
		if err != nil || len(key) != ed25519.PublicKeySize {
			return nil, errors.New("invalid update public key")
		}
		if _, exists := keys[parts[0]]; exists {
			return nil, errors.New("duplicate update public key")
		}
		keys[parts[0]] = ed25519.PublicKey(key)
	}
	if len(keys) == 0 {
		return nil, errors.New("update public key ring is empty")
	}
	return keys, nil
}

func readSignedManifest(path string) (SignedManifest, error) {
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 || info.Size() <= 0 || info.Size() > 16*1024 {
		return SignedManifest{}, errors.New("signed manifest must be a small regular file")
	}
	f, err := os.Open(path)
	if err != nil {
		return SignedManifest{}, err
	}
	defer f.Close()
	decoder := json.NewDecoder(io.LimitReader(f, 16*1024))
	decoder.DisallowUnknownFields()
	var manifest SignedManifest
	if err = decoder.Decode(&manifest); err != nil {
		return SignedManifest{}, err
	}
	var trailing any
	if err = decoder.Decode(&trailing); err != io.EOF {
		return SignedManifest{}, errors.New("manifest has trailing content")
	}
	return manifest, nil
}

func VerifySignedManifest(manifestPath, packagePath, expectedVersion, expectedPlatform, publicKeys string) (SignedManifest, error) {
	manifest, err := readSignedManifest(manifestPath)
	if err != nil {
		return SignedManifest{}, fmt.Errorf("read signed manifest: %w", err)
	}
	payload, err := manifest.canonicalPayload()
	if err != nil {
		return SignedManifest{}, err
	}
	if manifest.Version != expectedVersion || manifest.Platform != expectedPlatform {
		return SignedManifest{}, errors.New("signed manifest does not match requested update")
	}
	keys, err := parsePublicKeys(publicKeys)
	if err != nil {
		return SignedManifest{}, err
	}
	key, ok := keys[manifest.KeyID]
	if !ok || len(key) != ed25519.PublicKeySize {
		return SignedManifest{}, errors.New("unknown update signing key")
	}
	signature, err := base64.StdEncoding.DecodeString(manifest.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize || !ed25519.Verify(key, payload, signature) {
		return SignedManifest{}, errors.New("invalid update manifest signature")
	}
	info, err := os.Lstat(packagePath)
	if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 || info.Size() != manifest.Size {
		return SignedManifest{}, errors.New("package does not match signed size")
	}
	f, err := os.Open(packagePath)
	if err != nil {
		return SignedManifest{}, err
	}
	hash := sha256.New()
	_, copyErr := io.Copy(hash, f)
	closeErr := f.Close()
	if copyErr != nil || closeErr != nil || hex.EncodeToString(hash.Sum(nil)) != manifest.SHA256 {
		return SignedManifest{}, errors.New("package does not match signed digest")
	}
	return manifest, nil
}

func CreateSignedManifest(packagePath, version, platform, keyID string, privateKey ed25519.PrivateKey) (SignedManifest, error) {
	info, err := os.Lstat(packagePath)
	if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 {
		return SignedManifest{}, errors.New("package must be a regular file")
	}
	f, err := os.Open(packagePath)
	if err != nil {
		return SignedManifest{}, err
	}
	hash := sha256.New()
	_, copyErr := io.Copy(hash, f)
	closeErr := f.Close()
	if copyErr != nil || closeErr != nil {
		return SignedManifest{}, errors.New("could not hash package")
	}
	manifest := SignedManifest{
		Schema: ManifestSchema, KeyID: keyID, Version: version, Platform: platform,
		Architecture: "x86_64", Asset: info.Name(), Size: info.Size(),
		SHA256: hex.EncodeToString(hash.Sum(nil)),
	}
	payload, err := manifest.canonicalPayload()
	if err != nil {
		return SignedManifest{}, err
	}
	if len(privateKey) != ed25519.PrivateKeySize {
		return SignedManifest{}, errors.New("invalid Ed25519 private key")
	}
	manifest.Signature = base64.StdEncoding.EncodeToString(ed25519.Sign(privateKey, payload))
	return manifest, nil
}

func SigningKeyMatches(keyID string, privateKey ed25519.PrivateKey, publicKeys string) bool {
	keys, err := parsePublicKeys(publicKeys)
	if err != nil || len(privateKey) != ed25519.PrivateKeySize {
		return false
	}
	public, ok := privateKey.Public().(ed25519.PublicKey)
	return ok && public.Equal(keys[keyID])
}

func CompareManifestVersions(left, right string) (int, error) {
	a := manifestVersion.FindStringSubmatch(left)
	b := manifestVersion.FindStringSubmatch(right)
	if a == nil || b == nil {
		return 0, errors.New("invalid semantic version")
	}
	for i := 1; i <= 3; i++ {
		if len(a[i]) != len(b[i]) {
			if len(a[i]) < len(b[i]) {
				return -1, nil
			}
			return 1, nil
		}
		if a[i] < b[i] {
			return -1, nil
		}
		if a[i] > b[i] {
			return 1, nil
		}
	}
	return 0, nil
}

func SnapshotSignedPackage(manifestPath, packagePath, destinationDir, expectedVersion, expectedPlatform, publicKeys string) (string, string, error) {
	manifest, err := readSignedManifest(manifestPath)
	if err != nil {
		return "", "", err
	}
	payload, err := manifest.canonicalPayload()
	if err != nil || manifest.Version != expectedVersion || manifest.Platform != expectedPlatform {
		return "", "", errors.New("signed manifest does not match requested update")
	}
	keys, err := parsePublicKeys(publicKeys)
	if err != nil {
		return "", "", err
	}
	key, ok := keys[manifest.KeyID]
	if !ok || len(key) != ed25519.PublicKeySize {
		return "", "", errors.New("unknown update signing key")
	}
	signature, err := base64.StdEncoding.DecodeString(manifest.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize || !ed25519.Verify(key, payload, signature) {
		return "", "", errors.New("invalid update manifest signature")
	}
	source, err := os.Open(packagePath)
	if err != nil {
		return "", "", err
	}
	defer source.Close()
	info, err := source.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Size() != manifest.Size {
		return "", "", errors.New("package does not match signed size")
	}
	packageDestination := destinationDir + string(os.PathSeparator) + manifest.Asset
	destination, err := os.OpenFile(packageDestination, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return "", "", err
	}
	hash := sha256.New()
	written, copyErr := io.Copy(io.MultiWriter(destination, hash), source)
	syncErr := destination.Sync()
	closeErr := destination.Close()
	if copyErr != nil || syncErr != nil || closeErr != nil || written != manifest.Size || hex.EncodeToString(hash.Sum(nil)) != manifest.SHA256 {
		_ = os.Remove(packageDestination)
		return "", "", errors.New("package does not match signed digest")
	}
	manifestDestination := destinationDir + string(os.PathSeparator) + manifest.Asset + ".update.json"
	body, err := json.Marshal(manifest)
	if err != nil || !writeExclusiveFile(manifestDestination, append(body, '\n')) {
		_ = os.Remove(packageDestination)
		return "", "", errors.New("could not persist protected manifest")
	}
	return packageDestination, manifestDestination, nil
}

func writeExclusiveFile(path string, body []byte) bool {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return false
	}
	written, writeErr := f.Write(body)
	syncErr := f.Sync()
	closeErr := f.Close()
	return written == len(body) && writeErr == nil && syncErr == nil && closeErr == nil
}
