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

## Checked arithmetic for financial amounts

Financial amounts (fees, P&L, balances, stakes — anything denominated in a
Stellar 7-decimal `i128`) must not use raw `+`, `-`, `*`, `/` operators, since
those panic on overflow in debug builds and wrap or panic unpredictably
otherwise (Soroban release profile sets `overflow-checks = true`).

Use `stellar_swipe_common::Amount` instead:

```rust
use stellar_swipe_common::Amount;

let total = Amount::new(a).checked_add(Amount::new(b))?; // Result<Amount, AmountError>
let fee = principal.checked_mul_rate(fee_bps, 10_000)?;   // principal * fee_bps / 10_000
```

`Amount` intentionally has no `Add`/`Sub`/`Mul`/`Div` impls, so attempting
`amount_a + amount_b` is a compile error. Functions that perform financial
arithmetic should additionally carry `#[warn(clippy::arithmetic_side_effects)]`
on the function item — CI runs `cargo clippy --workspace --all-targets -- -D
warnings`, so any raw arithmetic introduced inside that function fails the
build (see `contracts/fee_collector/src/rebates.rs::record_trade_volume` and
`contracts/user_portfolio/src/queries.rs::compute_get_pnl` for examples).

This is scoped per-function rather than per-crate because the workspace sets
`clippy::all = "allow"` broadly (issue #599) — a crate-wide deny would also
flag unrelated, already-safe loop/index arithmetic across these large crates.

## Fuzz / property-based testing (signal_registry)

`signal_registry` validates untrusted signal input (price, expiry, rationale
length, tag count — see `validation::validate_signal_input` and the
`MAX_EXPIRY_SECONDS` / `MAX_RATIONALE_LEN` checks in `create_signal`). Two
layers of testing back that up beyond the fixed-input unit tests in `test.rs`:

### Property tests (run on every `cargo test`)

`contracts/signal_registry/src/tests/property_tests.rs` uses
[`proptest`](https://docs.rs/proptest) to assert `get_signal_quality_score` /
`calculate_quality_score` stay within `[0, 100]` across randomized execution
counts, adoption counts, stake amounts, and AI scores. No separate feature
flag or invocation is needed — it's a normal `[dev-dependencies]` crate, so
it runs as part of the existing `cargo test --workspace --all-targets` step
in CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) like every
other test in the crate.

### Fuzzing (local, not run in CI)

`contracts/signal_registry/fuzz/` is a [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz)
target (`fuzz_create_signal`) that generates arbitrary `(price, expiry_offset,
rationale bytes, tag count)` combinations and calls `create_signal` through
the generated `SignalRegistryClient::try_create_signal`, asserting the call
only ever completes as `Ok` or a typed `AdminError` — never an unwind panic
(a panic inside the contract call is caught by the Soroban host at the
invocation boundary and surfaced as an `Err`, so a crash here would indicate
either an issue outside that boundary or a genuine host regression).

To run it locally:

```bash
cargo install cargo-fuzz   # one-time
cd stellar-swipe/contracts/signal_registry/fuzz
cargo +nightly fuzz run fuzz_create_signal -- -max_total_time=60
```

The fuzz crate is a **detached workspace** (its own `[workspace]` /
`Cargo.lock`, separate from `stellar-swipe/Cargo.toml`) so `libfuzzer-sys`,
`arbitrary`, and the nightly sanitizer build it needs never become
dependencies of the deployed contract workspace or its `cargo-deny` /
reproducible-build checks. Its `Cargo.toml` pins every `soroban-*` crate (and
`ed25519-dalek`) to the exact versions in `stellar-swipe/Cargo.lock` — left
unpinned, a fresh resolve of `soroban-env-host`'s `ed25519-dalek = ">=2.0.0"`
requirement picks up `ed25519-dalek` 3.x, which breaks
`soroban-env-host`'s own `testutils` build (its `with_test_prng` helper
predates ed25519-dalek 3's `CryptoRng` bound). If `cargo fuzz build` ever
fails with a `ChaCha20Rng: CryptoRng` trait error, re-sync those pinned
versions with the current `stellar-swipe/Cargo.lock` and run
`cargo update -p ed25519-dalek@<new-version> --precise 2.2.0` (or whatever
version the parent lockfile carries) inside `fuzz/` to force the two
`ed25519-dalek` copies back into one.

Because fuzzing needs the nightly toolchain and cargo-fuzz's coverage
instrumentation, it is **not** part of CI — run it locally before merging
changes to `validation.rs` or `create_signal`'s input handling, and seed
`fuzz/corpus/fuzz_create_signal/` with any crash-reproducing input you find
so it's covered by future runs.

## Security review

Any PR that touches `contracts/` (or a shared crate consumed by contracts,
e.g. `contracts/common`) must be reviewed against
[`docs/security/release_security_checklist.md`](docs/security/release_security_checklist.md)
before merge — see the checklist in the PR template
(`.github/pull_request_template.md`). It covers four categories: logic,
access control, upgrades, and arithmetic, each with pointers to the relevant
background analysis under `docs/security/`.

The automated portion of that gate (formatting, clippy, tests, deployment
manifest validation, error-code discriminant checks) runs in
[`.github/workflows/security-release-gate.yml`](.github/workflows/security-release-gate.yml)
on every PR to `main` and on every `v*` release tag. It is in addition to,
not a replacement for, the human checklist review.

Once a change has passed review, see
[`deployments/README.md`](deployments/README.md) for how it flows into an
actual release: deployment manifests, versioning, and the validators that
run before a contract is deployed or upgraded.
