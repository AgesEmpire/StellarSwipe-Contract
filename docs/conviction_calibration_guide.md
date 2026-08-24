# Conviction Calibration Guide

## Overview

Conviction calibration allows the governance admin to fine-tune how conviction voting weights are calculated. By configuring penalty and reward parameters, the system can:

- **Penalize low-conviction votes**: Reduce the weight of votes that are younger than a configurable threshold, discouraging last-minute voting or vote manipulation.
- **Reward sustained commitment**: Add a bonus percentage to conviction for votes that have been held beyond the threshold, incentivizing long-term alignment.
- **Cap maximum conviction**: Place an absolute upper bound on any single vote's conviction weight, preventing whales from dominating proposals.

## Calibration Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `penalty_threshold_days` | `u64` | `0` | Votes younger than this many days receive a weight penalty. `0` disables penalty. |
| `penalty_multiplier` | `u64` | `1` | Denominator for penalty fraction (e.g. `2` = halve the weight). Must be ≥ 1. |
| `reward_bonus_pct` | `u64` | `0` | Percentage (0–100) added to conviction for votes ≥ threshold. `0` disables bonus. |
| `max_conviction_cap` | `i128` | `0` | Absolute cap on individual vote conviction. `0` = unlimited. |

## How It Works

1. **Base conviction** is computed as: `tokens * sqrt(days_elapsed) / 1000`
2. **Penalty** (if threshold > 0 and vote age < threshold): `conviction -= conviction / penalty_multiplier`
3. **Reward bonus** (if bonus_pct > 0 and vote age >= threshold): `conviction += conviction * reward_bonus_pct / 100`
4. **Cap** (if max_conviction_cap > 0 and conviction > cap): `conviction = max_conviction_cap`

## Governance Outcomes

| Configuration | Effect |
|---------------|--------|
| Penalty enabled | Short-lived votes contribute less, making it harder to rush proposals through. |
| Reward enabled | Long-term stakers get proportionally more influence, aligning voting power with sustained interest. |
| Cap enabled | Prevents single large voter from dominating a proposal, encouraging broader participation. |
| All disabled (default) | Original linear sqrt conviction curve — no adjustments. |

## Admin Functions

### Read current config
```rust
let config: ConvictionCalibration = client.conviction_calibration();
```

### Set new config (admin only)
```rust
let config = ConvictionCalibration {
    penalty_threshold_days: 7,
    penalty_multiplier: 2,
    reward_bonus_pct: 10,
    max_conviction_cap: 50_000,
};
client.set_conviction_calibration(&admin, &config);
```

## Example Scenarios

**Scenario 1: Anti-sniping**
- Set `penalty_threshold_days = 3`, `penalty_multiplier = 2`
- Votes within the first 3 days have their conviction halved
- Prevents last-minute vote dumps from swinging proposals

**Scenario 2: Loyalty bonus**
- Set `penalty_threshold_days = 14`, `reward_bonus_pct = 25`
- Voters who have held for 2+ weeks earn 25% more conviction weight
- Rewards consistent participation

**Scenario 3: Whale cap**
- Set `max_conviction_cap = 100_000`
- No single vote can contribute more than 100k conviction, regardless of tokens staked
- Encourages wider distribution of voting power
