# Explicit, Auditable Reputation Decay & Slashing

## Problem
Reputation and slashing behavior in `stake_vault` and `signal_registry` are
central to trust/risk management but currently lack operational clarity and
consistent, auditable events across both contracts.

## Proposed Solution
- Define a shared `ReputationEvent` enum in `contracts/common` with explicit
  variants: `Decayed { provider, old_score, new_score, ledger }`,
  `Slashed { provider, amount, reason, ledger }`,
  `Restored { provider, amount, ledger }`.
- Both `stake_vault` and `signal_registry` emit the same event shapes so
  off-chain consumers can correlate reputation changes across subsystems
  without bespoke parsing per contract.
- Make decay explicit and deterministic: a `decay_rate_bps` config value and a
  `apply_decay(provider)` entrypoint (or scheduled hook) that computes decay
  based on ledger time elapsed since `last_activity_ledger`, rather than an
  implicit/opaque calculation buried in unrelated logic.
- Slashing paths (`slash_provider`) must always emit a `Slashed` event with an
  explicit `reason` code (reusing the typed error/reason discriminants
  pattern already established for signal registry error codes) so audits can
  reconstruct why a slash occurred.

## Benefits
- Consistent, typed events make reputation/slashing auditable off-chain.
- Deterministic decay formula removes ambiguity about how scores evolve.
- Shared event vocabulary simplifies building trust/risk dashboards.

## Next Steps
- Add `ReputationEvent` to `contracts/common`.
- Update `stake_vault` and `signal_registry` slashing/decay code paths to emit
  the shared event shape.
- Add unit tests per flow asserting the correct event + reason is emitted for
  each slashing/decay scenario.
