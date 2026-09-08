# Update signing

Protected Windows installations accept only release packages authorized by an
Ed25519 manifest. The private key belongs only in the GitHub Actions secret;
release binaries contain the public key ring.

Generate the initial key once on a trusted machine:

```sh
cd updater
go run ./cmd/harbor-update-keygen \
  -key-id release-1 \
  -private-output /secure/offline/harbor-update-release-1.seed
```

The command prints the values for the repository variables
`HARBOR_UPDATE_KEY_ID` and `HARBOR_UPDATE_PUBLIC_KEYS`. Set the
`HARBOR_UPDATE_SIGNING_KEY` repository secret to the single base64 line in the
private seed file. Never commit that file.

The protected broker is installer-owned and is not replaced by the portable
ZIP. For rotation, first publish an installer whose
`HARBOR_UPDATE_PUBLIC_KEYS` contains both the old and new
`key-id:base64-public-key` entries, and require that installer as the rotation
release. Only then switch `HARBOR_UPDATE_KEY_ID` and
`HARBOR_UPDATE_SIGNING_KEY` to the new key. Keep the old public key until all
supported installations have crossed that installer release.

The release workflow fails closed when signing configuration is absent or the
private key does not match the selected public-key entry.
