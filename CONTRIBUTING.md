# Contributing to StellarSwipe-Contract

## Scaffold a new contract crate

Use the scaffold generator to create a new contract crate already wired with the
shared **Pausable**, **Initializable**, and **StorageTrait** conventions:

```bash
# From the repository root
./stellar-swipe/scripts/scaffold_contract.sh <contract_name>
```

### Example

```bash
./stellar-swipe/scripts/scaffold_contract.sh reward_distributor
```

This creates:

```
stellar-swipe/contracts/reward_distributor/
├── Cargo.toml          # depends on soroban-sdk + stellar-swipe-common
└── src/
    ├── lib.rs          # initialize / pause / unpause / storage_write / storage_read
    └── tests.rs        # starter tests covering init, pause, storage round-trip
```

The workspace `Cargo.toml` is updated automatically to include the new crate.

### Verify the scaffold

```bash
cd stellar-swipe
cargo test   -p stellar-swipe-reward-distributor
cargo clippy -p stellar-swipe-reward-distributor -- -D warnings
```

Both should pass with no manual fixes required.

### What the scaffold includes

| Feature | Implementation |
|---|---|
| Initializable guard | `initialize()` panics/returns error on double-init |
| Pausable | `pause()` / `unpause()` / `is_paused()` with events |
| StorageTrait pattern | `storage_write(key, value)` / `storage_read(key)` blocked while paused |
| Starter test file | `tests.rs` with 5 tests covering all three features |

Extend `DataKey` and `{ContractName}Error` with your contract-specific variants
before adding business logic.

---

## Clippy policy

CI runs `cargo clippy --workspace --all-targets -- -D warnings`.  Any clippy
warning in a PR will fail the build.

**Workspace-wide suppressed lints** (see `[workspace.lints.clippy]` in
`stellar-swipe/Cargo.toml`):

| Lint | Reason |
|---|---|
| `too_many_arguments` | Soroban contract functions must receive all state as explicit arguments; the SDK pattern naturally exceeds the default limit. |
| `type_complexity` | Complex nested types are sometimes unavoidable in `no_std` without the standard type-alias trait idiom. |

**Suppressing any other lint** requires a narrowly-scoped attribute
(`#[allow(...)]` on the specific item, not the whole crate) with a comment
explaining *why*.  Open a PR adding it to the workspace-level table above if the
suppression applies across the whole project.

---

## Adding or changing `#[contracterror]` codes

Contract error codes are a public API.  Clients (SDKs, wallets, monitoring
tools) match on the numeric value, so the mapping between variant name and
number must be stable.

### Allowed

- Adding a **brand-new** variant with a **never-before-used** number.

### Forbidden

- Changing the number of an existing variant (renumbering).
- Reusing a number that previously belonged to a different variant.
- Removing a variant and later re-adding it with a different number.

### How to add a new error code

1. Add the variant to the enum with a fresh number (higher than all existing ones
   is a safe default).
2. Update the baseline:
   ```bash
   cd stellar-swipe
   python3 scripts/check_error_codes.py --update-baseline
   ```
3. Commit the updated file(s) under `error-baselines/` together with your code
   change.
4. CI will pass once the baseline matches the source.

### Deprecating a code (rare)

Never remove or renumber a deprecated variant.  Instead:

1. Add a doc comment: `/// DEPRECATED – use FooBar instead.`
2. Keep the numeric value unchanged so no future variant can accidentally reuse it.
3. The variant may be removed from the enum only if all clients have migrated and
   a major version bump signals the breaking change.

---

## Adding dependencies (cargo-deny)

All crate dependencies are checked by `cargo-deny` in CI.  The policy lives in
`stellar-swipe/deny.toml`.

### Rules at a glance

| Rule | Policy |
|---|---|
| License | Must be in the `allow` list (Apache-2.0, MIT, ISC, BSD variants, Unicode, CC0, 0BSD). |
| Source | Published on crates.io only.  No git deps (unless pinned to an immutable commit SHA). |
| Banned crates | `openssl`, `openssl-sys`, `ring` (incompatible with `wasm32-unknown-unknown`). |

### Adding a new dependency

1. Ensure the crate is published on crates.io.
2. Verify its license appears in `deny.toml`'s `allow` list.
3. Run `cargo deny --manifest-path stellar-swipe/Cargo.toml check` locally.
4. If the check fails, either choose a compliant alternative or add a documented
   exception in `deny.toml` (under `[licenses.exceptions]` or `[[bans.skip]]`)
   in the same PR.

### Requesting a policy exception

Add the exception entry to `deny.toml` with a comment explaining the business
reason and the security review outcome.  Include it in the PR description and
tag a maintainer for explicit approval.
