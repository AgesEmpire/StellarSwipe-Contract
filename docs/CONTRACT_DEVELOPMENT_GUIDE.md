# Contract Development Guide

This guide covers building, testing, and releasing the Soroban smart contracts in
this repository. It focuses on the release process and, in particular, on the
WASM reproducibility check that gates every contract release, and on the panic
detection and forbidden API lint that gate every contract build.

## Toolchain

Contract builds are pinned to a single supported toolchain. The release check
records the exact toolchain, build target, and dependency lock state so that a
build can be reproduced later.

- **Rust toolchain:** the version pinned in `rust-toolchain.toml` (currently the
  `stable` channel with the `wasm32-unknown-unknown` target).
- **Build target:** `wasm32-unknown-unknown`.
- **Dependency lock state:** `Cargo.lock` is committed and must not change during
  a release build. The lock file hash is captured alongside the artifact hashes.

Install the target once with:

```sh
rustup target add wasm32-unknown-unknown
```

## Building contract WASM

Build the release artifact for a contract with:

```sh
cargo build \
  --manifest-path contracts/signal_registry/Cargo.toml \
  --target wasm32-unknown-unknown \
  --release
```

The resulting artifact is written to
`target/wasm32-unknown-unknown/release/<contract>.wasm`.

## Panic detection and forbidden API lint

Before a contract can be deployed, CI runs a static check that rejects panic
paths and forbidden non-deterministic or unsupported APIs. The check covers
**all contract crates** under `contracts/` and runs for **both the debug and
release profiles**, so a forbidden pattern cannot slip through by only building
one profile.

### Forbidden patterns

The check fails the build when a contract crate contains any of the following:

- **Panic paths:** `panic!`, `unreachable!`, `todo!`, `unimplemented!`,
  `assert!`, `assert_eq!`, `assert_ne!`, and `unwrap()` / `expect()` calls.
  These abort the contract and are not recoverable on-chain.
- **Non-deterministic APIs:** `std::time`, `SystemTime`, `Instant`,
  `std::env::var`, `std::env::args`, `rand::`, and `getrandom::`. Contract
  execution must be deterministic across validators.
- **Unsupported APIs:** `std::fs`, `std::net`, `std::process`, `std::thread`,
  and `std::io`. These are unavailable in the Soroban host environment.

### Running the check locally

Run the same check CI runs, for every contract crate and both profiles:

```sh
./scripts/check-contract-panics.sh
```

The script scans each crate under `contracts/` and reports every offending
source location as `path:line: <pattern>`. It exits non-zero when any forbidden
pattern is found.

### Allowlist for intentional exceptions

Intentional exceptions must be explicit and reviewed. Add an entry to
`contracts/panic-allowlist.toml` with the source location, the pattern, and a
justification plus the reviewer who approved it:

```toml
[[allow]]
path = "contracts/signal_registry/src/lib.rs"
pattern = "unwrap"
justification = "Infallible: value is set immediately above."
reviewer = "@maintainer"
```

Allowlist entries are reviewed like any other code change. An entry without a
justification and reviewer is rejected by the check.

### Test fixture

The check is itself tested by a fixture that intentionally contains a forbidden
pattern (`contracts/test-fixtures/panic-fixture/src/lib.rs`). The fixture test
asserts that the check **fails** on the fixture, proving the check catches a
forbidden pattern rather than silently passing.

### CI behavior

When the check fails, CI reports, for each violation:

- the offending **source location** (`path:line`),
- the **pattern** that matched,
- **remediation guidance** pointing back to this section.

A panic path or forbidden API is treated as a release blocker.

## Reproducibility check

The release check builds each contract WASM **twice** from the same source and
compares the resulting hashes. A release is only accepted when the two builds
produce byte-identical artifacts.

For each contract the check:

1. Builds the WASM from a clean target directory.
2. Records the SHA-256 hash of the artifact.
3. Rebuilds from the same source and lock state.
4. Records the second SHA-256 hash.
5. Fails if the two hashes differ.

Alongside the hashes, the check captures the metadata needed to reproduce the
build:

- the Rust toolchain version (`rustc --version`),
- the build target (`wasm32-unknown-unknown`),
- the `Cargo.lock` hash (dependency lock state),
- the artifact path and its SHA-256 hash for each build.

### CI behavior

The reproducibility check runs as part of CI. When the two builds disagree, CI
**fails** and prints actionable diagnostics:

- the contract name and artifact path,
- the first and second SHA-256 hashes,
- the toolchain version and target used,
- the `Cargo.lock` hash,
- a pointer to this section for investigation steps.

A non-reproducible build is treated as a release blocker.

## Investigating hash differences

If the reproducibility check reports differing hashes, work through the
following steps before re-running CI.

1. **Confirm the toolchain.** Run `rustc --version` and compare it with the
   version reported in the CI diagnostics. A different toolchain (or a locally
   installed nightly) is the most common cause of hash drift. Reinstall the
   pinned toolchain with `rustup toolchain install` and rebuild.

2. **Confirm the target.** Ensure `wasm32-unknown-unknown` is installed and that
   no other target was used. Building for a host target or a different WASM
   target produces different bytes.

3. **Confirm the lock state.** Run `git status Cargo.lock` and compare the
   `Cargo.lock` hash with the one in the diagnostics. An uncommitted or updated
   lock file changes dependency versions and therefore the artifact.

4. **Check for non-deterministic inputs.** Look for anything that embeds a
   timestamp, absolute path, host name, random value, or environment variable
   into the build. These make the output non-reproducible and must be removed or
   made deterministic.

5. **Rebuild from a clean state.** Remove the target directory
   (`cargo clean`) and rebuild twice locally. If the hashes still differ, the
   non-determinism is in the source or build configuration, not in stale
   artifacts.

6. **Compare artifacts.** Use `sha256sum` on both artifacts and, if needed,
   `wasm-objdump` or `wasm2wat` to diff the two builds and locate the section
   that changed.

Once the cause is fixed, commit the change and re-run the release check. The
build must produce identical hashes across repeated runs before the release can
proceed.
