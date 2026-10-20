# Source Verification

Every deployed StellarSwipe contract WASM embeds two metadata entries that let
a third party independently verify that a specific on-chain binary matches the
source code used to produce it:

| Metadata key  | Value                                                            |
|---------------|------------------------------------------------------------------|
| `SourceHash`  | SHA-256 of the reproducible source snapshot (see below)         |
| `GitCommit`   | Full Git commit SHA at build time                               |

## How the hash is computed

The hash is computed by `stellar-swipe/scripts/embed_source_hash.sh` before
`cargo build` runs.  It covers every `*.rs` and `*.toml` file tracked by Git
under `stellar-swipe/`, sorted alphabetically so the result is identical on
any platform.

```
sha256( sha256(file_1) || sha256(file_2) || … )
```

where each inner hash line is in `sha256sum` format (`<hash>  <path>`).

## Reading the metadata from a deployed WASM

```bash
# Inspect an optimised WASM produced by the release build
stellar contract inspect --wasm target/wasm-optimized/<contract>.wasm
```

The output includes a `Meta` section.  Look for `key="SourceHash"` and
`key="GitCommit"`.

## Third-party verification (step-by-step)

1. **Record the metadata** from the deployed contract.

   ```bash
   stellar contract inspect --wasm <contract>.wasm
   # Note the SourceHash and GitCommit values.
   ```

2. **Fetch the exact source snapshot.**

   ```bash
   git clone https://github.com/StellarSwipe/StellarSwipe-Contract.git
   cd StellarSwipe-Contract
   git checkout <GitCommit>
   ```

3. **Recompute the source hash.**

   ```bash
   cd stellar-swipe
   source ./scripts/embed_source_hash.sh
   echo "$STELLAR_SOURCE_HASH"
   ```

4. **Compare.** The printed hash must equal the `SourceHash` embedded in the
   WASM.  Any discrepancy means the deployed binary was not produced from the
   referenced commit without modification.

5. **Optional: rebuild and diff the WASM.**  Because the build uses a pinned
   Rust toolchain and deterministic Cargo flags, a reproducible build should
   produce a byte-identical WASM.

   ```bash
   # Install the exact toolchain version used for the release (see rust-toolchain.toml).
   ./scripts/build.sh
   diff target/wasm-optimized/<contract>.wasm <reference-wasm>
   ```

## CI enforcement

`scripts/build.sh` sources `embed_source_hash.sh` and aborts with a non-zero
exit code if `STELLAR_SOURCE_HASH` is empty.  The CI release step therefore
fails before producing any artifact with missing metadata.

## Requesting an exception

If a build cannot produce a reproducible hash (e.g. a one-off emergency
hotfix), open a PR that explains why, tags the `security` label, and
includes manual verification evidence.  A merge requires explicit approval from
two maintainers.
