# Deployments

This directory holds deployment tooling and release artifacts for the Soroban
contracts in this repository.

## Contract release artifact manifest

Every deployable Soroban WASM artifact must ship with a matching entry in the
release manifest (`deployments/release-manifest.json`). Each entry records the
contract name, interface version, checksum, and source revision so that CI can
reject incomplete or mismatched manifests before a release is published.

### Manifest entry shape

```json
{
  "contract": "token",
  "interfaceVersion": "1.0.0",
  "checksum": "sha256:<hex>",
  "sourceRevision": "<git-commit-sha>",
  "artifact": "target/wasm32-unknown-unknown/release/token.wasm"
}
```

### Verification command

Run the manifest validation before publishing a release. It checks that every
built WASM artifact has a manifest entry with a matching contract name,
interface version, checksum, and source revision, and fails if any entry is
missing, mismatched, or if a WASM file has been tampered with (checksum
mismatch):

```sh
./scripts/verify-release-manifest.sh
```

The command exits non-zero when:

- a deployable WASM artifact has no manifest entry,
- a manifest entry references a missing artifact,
- the contract name or interface version does not match the artifact,
- the recorded checksum does not match the built WASM (tampered artifact),
- the source revision does not match the current release revision.

CI runs this command as part of the release workflow, so an incomplete or
mismatched manifest blocks the release.
