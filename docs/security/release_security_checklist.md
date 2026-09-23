# Release Security Checklist

**Status:** Active  
**Scope:** Every PR that touches `contracts/` (or shared crates consumed by
contracts), and every tagged release (`v*`)  
**Last updated:** 2026-07-27

## Purpose

This is the **recurring, per-release security review gate** for StellarSwipe.
A reviewer works through it on every contract-touching pull request, and it
is re-affirmed at tag time before a release ships. It is deliberately
lightweight — a checklist a human ticks during code review, not a sign-off
form.

This is **not** the same document as
[`pre_mainnet_checklist.md`](./pre_mainnet_checklist.md), which is a
one-time, heavier sign-off gate completed once before the very first mainnet
launch (audit findings, incident response readiness, multi-sig rollout,
executive sign-off, etc.). Use this checklist for ongoing iteration —
day-to-day PRs and routine releases after mainnet launch. Use
`pre_mainnet_checklist.md` only for the initial mainnet go-live (or a
comparably high-impact re-launch).

## How to use this checklist

1. Any PR that changes code under `contracts/` (or a shared crate consumed by
   contracts, e.g. `contracts/common`) must have this checklist reviewed
   before merge — see the PR template
   (`.github/pull_request_template.md`).
2. Go through the four categories below. For each item, check the box only
   if you actually verified it for **this** change — not out of habit.
3. Items that don't apply to a given PR (e.g. no upgrade-relevant storage
   change) can be left unchecked with a one-line note in the PR ("N/A — no
   storage changes in this PR") rather than silently ticked.
4. The automated portion of this gate runs in
   [`.github/workflows/security-release-gate.yml`](../../.github/workflows/security-release-gate.yml)
   on every PR to `main` and on every `v*` release tag. It covers the parts
   that are mechanically checkable (formatting, lints, tests, deployment
   manifest validity, error-code discriminant integrity). It does **not**
   replace the human review below — logic, access-control intent, and
   upgrade-safety judgement calls still require a person.

---

## 1. Logic

Business-logic correctness, invariants, and state-machine transitions.

- [ ] **Invariants hold across the change.** Any protocol invariant the
      touched function relies on (balances non-negative, position states
      only move forward, totals reconcile) still holds after the diff, not
      just on the happy path.
- [ ] **State-machine transitions are validated.** New or modified state
      transitions (e.g. pending → active → closed) reject invalid
      transitions explicitly rather than relying on caller discipline.
- [ ] **Edge cases and boundary values are covered by tests.** Zero amounts,
      empty collections, first/last element, exactly-at-threshold values.
- [ ] **Error paths leave state unchanged.** A function that returns an
      `Err` (or panics) does not leave partial writes behind — see
      [`security_model.md`](./security_model.md) for the trust assumptions
      this depends on.
- [ ] **Oracle- or price-dependent logic is reviewed against known attack
      surface.** See
      [`flash_loan_analysis.md`](./flash_loan_analysis.md) for the existing
      inventory of price reads and which ones feed financial decisions.
- [ ] **Cross-contract call ordering is reentrancy-safe.** Any new
      cross-contract call that moves tokens or triggers a callback follows
      the guard pattern documented in
      [`reentrancy_analysis.md`](./reentrancy_analysis.md) (lock-before-call,
      clear-on-all-paths).
- [ ] **Ordering/front-running exposure is considered** for any change to
      trade execution or signal handling — see
      [`front_running_analysis.md`](./front_running_analysis.md).

## 2. Access control

`require_auth()` usage, admin/governance/timelock privilege boundaries,
unauthorized-caller checks.

- [ ] **Every privileged or user-scoped entrypoint calls `require_auth()`
      (or `require_auth_for_args()`) on the correct address**, not on an
      address the caller can freely substitute.
- [ ] **Admin/governance-only functions actually check the admin/governance
      role**, not just that *some* auth was provided. Compare against the
      existing role inventory in
      [`security_model.md`](./security_model.md) (Contract Admin, Multisig
      Signers, Guardian, Oracle Operators, Governance Actors).
- [ ] **New admin-key writes follow the existing transfer pattern**
      (init-once, then `pending → accept` with expiry and
      `require_auth()`), consistent with the audited paths in
      [`privilege_escalation_analysis.md`](./privilege_escalation_analysis.md).
      Any new path that writes an admin/role storage key should be added to
      that inventory.
- [ ] **Negative tests exist**: at least one test proves an unauthorized
      caller is rejected for each new privileged function.
- [ ] **Timelock/governance-gated actions cannot be reached through a
      side door** (e.g. a "helper" function that performs the same
      state change without the timelock/proposal check).
- [ ] **No function silently widens who can call it** relative to the
      pre-change behavior (e.g. loosening `require_auth()` from a specific
      address to "any caller" without an explicit, reviewed reason).

## 3. Upgrades

Storage migration safety, versioning, and backward compatibility of storage
keys.

- [ ] **New or changed storage keys don't collide with existing ones.**
      Follow the enum-variant-naming convention documented in
      [`storage_key_analysis.md`](./storage_key_analysis.md); if a new
      `#[contracttype]` key enum or variant is added, sanity-check it
      against that inventory.
- [ ] **Changing an existing storage layout includes a migration path**
      (or an explicit justification for why old data can be left as-is)
      rather than assuming a field is always present going forward.
- [ ] **Contract interface version is bumped when required.** Follow
      `stellar-swipe/docs/shared_version_upgrade_rules.md`: bump the
      relevant `*_VERSION` constant and `min_version_for` for breaking
      changes (removed/renamed method, changed storage layout, changed
      parameter semantics, removed `#[contracttype]` variant); no bump
      needed for additive, backward-compatible changes.
- [ ] **Cross-contract callers re-verify the callee version** per
      `stellar-swipe/docs/CROSS_CONTRACT_INTERFACE_VERSIONING.md` if this
      PR changes a callee's interface — add/update the caller-side client
      wrapper's expected version constant and a regression test that
      proves an incompatible callee is rejected.
- [ ] **WASM ABI export check is clean or intentionally acknowledged.**
      CI's `check_wasm_exports.py` step (in `ci.yml`) flags removed exports
      or changed signatures; a red result here means this PR is a breaking
      ABI change and needs the corresponding `abi-baselines/*.breaking.txt`
      acknowledgement, not a silent baseline update.
- [ ] **Deployment manifest changes are internally consistent** — versions
      and `depends_on` entries in any touched `deployments/*.manifest.json`
      satisfy `validate_deployment_manifest.py` (see
      [`deployments/README.md`](../../deployments/README.md)).

## 4. Arithmetic

Overflow/underflow, checked-amount usage, fee rounding.

- [ ] **All financial-amount arithmetic goes through
      `stellar_swipe_common::Amount`'s `checked_*` methods**, not raw `+`,
      `-`, `*`, `/` on `i128` — see
      `stellar-swipe/contracts/common/src/checked_amount.rs` and the
      "Checked arithmetic for financial amounts" section of
      [`CONTRIBUTING.md`](../../CONTRIBUTING.md).
- [ ] **New functions that do financial arithmetic on raw `i128` (migration
      code, defense-in-depth paths) carry
      `#[warn(clippy::arithmetic_side_effects)]`** so CI's `-D warnings`
      clippy pass catches accidental unchecked math, per the existing
      examples in `contracts/fee_collector/src/rebates.rs` and
      `contracts/user_portfolio/src/queries.rs`.
- [ ] **Division/rounding direction is deliberate and documented.** Fee and
      pro-rata calculations should default to floor division in the
      user-favorable direction unless there's a documented reason
      otherwise — see
      [`fee_rounding_analysis.md`](./fee_rounding_analysis.md) for the
      existing rounding-direction table and dust analysis.
- [ ] **No unwithdrawable dust is introduced.** If a new division/rounding
      path is added, confirm the remainder either stays with the payer or
      is provably swept somewhere accounted-for, rather than getting stuck
      in the contract.
- [ ] **Overflow/underflow tests exist for new arithmetic**, including
      near-`i128::MAX`/`MIN` and zero-denominator cases where relevant.

---

## Relationship to automated checks

| Category | Human review (this doc) | Automated check |
|---|---|---|
| Logic | Invariants, state machine, edge cases | `cargo test --workspace --all-targets` |
| Access control | Role/intent correctness | `cargo clippy` (misuse lints) + negative tests in the test suite |
| Upgrades | Migration safety, judgement on version bumps | `check_wasm_exports.py`, `check_error_codes.py`, `validate_deployment_manifest.py` (all run in `ci.yml`; the error-code and manifest checks are also re-run in the release gate) |
| Arithmetic | Rounding direction, dust reasoning | `cargo clippy --workspace --all-targets -- -D warnings` (via `clippy::arithmetic_side_effects` where enabled), `cargo test` |

The automated side of the gate lives in
[`.github/workflows/security-release-gate.yml`](../../.github/workflows/security-release-gate.yml).
It cannot evaluate judgement calls (Is this rounding direction correct? Is
this the right role for this action?) — that's what this checklist is for.

## See also

- [`pre_mainnet_checklist.md`](./pre_mainnet_checklist.md) — one-time
  mainnet-launch sign-off gate (heavier, includes named sign-offs).
- [`security_model.md`](./security_model.md) — trust assumptions and
  privileged-role inventory.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — contributor guide, including
  checked-arithmetic conventions.
- [`deployments/README.md`](../../deployments/README.md) — how a reviewed
  change flows into an actual deployment/release.
