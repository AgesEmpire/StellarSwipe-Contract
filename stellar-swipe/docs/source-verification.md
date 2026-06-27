# On-Chain Source Verification

Every StellarSwipe WASM deployed from the official CI pipeline embeds a
`source_hash` field in its contract metadata.  This hash lets a third party
verify that a running contract was compiled from the exact source snapshot that
produced it — without trusting the build team.

---

## How the hash is produced

`scripts/build.sh` computes a SHA-256 digest before invoking `cargo build`:

```text
find . \( -path ./target -o -path ./.git \) -prune \
  -o \( -name "*.rs" -o -name "Cargo.toml" -o -name "Cargo.lock" \) -print \
| sort \
| xargs sha256sum \
| sha256sum
```

The result is a deterministic 64-character hex string that covers every `.rs`
source file and every `Cargo.toml` / `Cargo.lock` in the workspace.  It is
exported as `SOURCE_HASH` and picked up at compile time by each contract's
`build.rs`, which forwards it to the Rust compiler via `cargo:rustc-env`.

Each contract's `lib.rs` then embeds it with:

```rust
soroban_sdk::contractmeta!(key = "source_hash", val = env!("SOURCE_HASH"));
```

CI rejects any release build where `SOURCE_HASH` is empty (the build script
exits non-zero before reaching `cargo build`).

---

## Reading the hash from a deployed contract

Using the [Stellar CLI](https://github.com/stellar/stellar-cli):

```bash
stellar contract info --id <CONTRACT_ID> --network testnet
```

Look for the `source_hash` key in the metadata output.

Using the JavaScript SDK:

```ts
import { Contract } from "@stellar/stellar-sdk";
const info = await server.getContractData(contractId, ...);
```

(Refer to the SDK docs for the exact `contractInfo` / `getLedgerEntries` call.)

---

## Reproducing and verifying a build

### Prerequisites

- Rust toolchain matching `rust-toolchain.toml` (or the version pinned in CI)
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- Stellar CLI: see [install docs](https://developers.stellar.org/docs/tools/cli/install-cli)

### Steps

1. **Check out the exact commit** referenced by the hash

   The `source_hash` in contract metadata is the SHA-256 of the source files at
   build time.  Find the matching git commit by looking at the CI job that
   produced the WASM (each release CI run logs the hash).

   ```bash
   git checkout <commit-sha>
   ```

2. **Reproduce the source hash locally**

   ```bash
   cd stellar-swipe
   find . \( -path ./target -o -path ./.git \) -prune \
     -o \( -name "*.rs" -o -name "Cargo.toml" -o -name "Cargo.lock" \) -print \
     | sort | xargs sha256sum | sha256sum | cut -d' ' -f1
   ```

   This must match the `source_hash` value read from the deployed contract.

3. **Build the WASM**

   ```bash
   cd stellar-swipe
   ./scripts/build.sh
   ```

   The build script automatically computes the same hash and embeds it.

4. **Compare WASM hashes**

   Fetch the official WASM artifact from the CI release (attached to the GitHub
   Actions run) and compare:

   ```bash
   sha256sum target/wasm-optimized/<contract>.wasm
   sha256sum <downloaded-artifact>/<contract>.wasm
   ```

   Identical hashes confirm the deployed WASM was produced from that exact source.

---

## CI enforcement

The `Build optimized Soroban WASM` step in `.github/workflows/ci.yml` calls
`scripts/build.sh`, which aborts with a non-zero exit code if `SOURCE_HASH`
cannot be computed.  This guarantees that no WASM artifact is uploaded without
an embedded source hash.

---

## Future: IPFS / Arweave CID

The current approach embeds a *local* hash.  To enable fully trustless
verification without access to the original git repository, a future iteration
may:

1. Create a reproducible source tarball (`git archive HEAD | gzip > source.tar.gz`).
2. Pin it to IPFS or Arweave and embed the CID instead of (or alongside) the SHA-256.
3. Update this document with the exact pinning command and gateway URL.
