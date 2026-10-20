# Configurable Stake/Unstake Cooldown Periods

## Problem
Without cooldowns, rapid stake and unstake activity can create manipulation
opportunities and make reward and slashing flows harder to reason about.

## Proposed Solution
- Add a `cooldown_secs` field to vault config (governance-settable, per-asset).
- Track `last_stake_ts` / `last_unstake_ts` per account in storage.
- On `stake`/`unstake`, reject the call if `now < last_action_ts + cooldown_secs`.
- Emit a `CooldownRejected` event with the remaining wait time for observability.
- Default cooldown: 24h for unstake, 1h for stake; both overridable via
  governance-gated `set_cooldown(asset, stake_secs, unstake_secs)`.

## Rollout
1. Add storage keys + config getters/setters (no behavior change, flag off).
2. Enforce checks behind a feature flag.
3. Enable by default after one release cycle of monitoring.
