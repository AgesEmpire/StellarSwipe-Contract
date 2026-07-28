# Automated ABI/Storage Compatibility Gate

## Problem
Contract evolution can unintentionally break existing integrations or
upgrade paths. A strong pre-merge gate would reduce regressions.

## Proposed Solution
- Generate and commit a versioned contract spec/ABI snapshot (e.g. via
  `soroban contract bindings` / spec-xdr extraction) per contract under
  `spec/<contract>/<version>.json`.
- Add a CI job (`compat-check.yml`) that, on every PR:
  - Regenerates the current spec from source.
  - Diffs it against the latest committed snapshot.
  - Fails the build on breaking changes: removed/renamed functions, changed
    argument types/order, removed storage keys, or changed storage value
    schemas — unless the PR bumps the contract's major version and includes
    an explicit migration note.
- Non-breaking additive changes (new optional fn, new storage key) pass
  automatically and update the snapshot.

## Rollout
1. Add spec snapshot generation script + baseline snapshots for existing
   contracts.
2. Add CI diff check in warn-only mode.
3. Flip to required/blocking after a grace period.
