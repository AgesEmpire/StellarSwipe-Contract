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

## Contract interface (ABI) changes

Every Soroban contract's exported function names and `contractspecv0` hash are
snapshotted in [`stellar-swipe/abi-baselines/`](stellar-swipe/abi-baselines/).
CI detects and reports any ABI difference automatically, posting a
[**WASM ABI Export Diff Report**](#) as a PR comment.

### Adding new exports (non-breaking)

Add the new function, build, and run CI. The script detects the new exports,
updates the baseline, and exits with code 2 (non-failing). Commit the updated
`abi-baselines/<contract>.json` file alongside your change.

### Changing or removing exports (breaking)

A function removal or a parameter-name/type/order change is a **breaking ABI
change** — CI fails with exit code 1 unless you explicitly acknowledge it:

1. Verify that the migration path covers all existing stored data and
   cross-contract callers.
2. Create `abi-baselines/<contract>.breaking.txt` with a short reason
   (one line is fine).
3. Re-run CI — the script updates the baseline and succeeds.
4. Commit both the updated `abi-baselines/<contract>.json` and the
   `.breaking.txt` file together.
5. Remove `.breaking.txt` in the very next PR so accidental future breaks
   are still caught.

> The PR comment makes it easy for reviewers to see exactly which exports
> changed and whether the change was intentional.
