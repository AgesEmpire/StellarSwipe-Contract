# Reputation Decay & Stale-Score Guide

## Overview

The governance reputation system now includes **decay scheduling** and **stale-score detection** to ensure that provider reputation ages gracefully and stale contributions do not inflate influence. This document explains how reputation age affects voting power in `cast_reputation_weighted_vote`.

---

## Decay Schedule Tiers

Reputation points decay according to a **tier-based schedule**. Each user is assigned a tier based on their total participation actions (proposals created + votes cast + committee memberships + delegations received).

| Tier     | Minimum Actions | Decay Rate (per inactive day) | Grace Period | Max Decay Days |
|----------|-----------------|-------------------------------|--------------|----------------|
| Bronze   | 0               | 0.5% (50 BPS)                 | 30 days      | 90 days        |
| Silver   | 50              | 0.3% (30 BPS)                 | 60 days      | 180 days       |
| Gold     | 200             | 0.2% (20 BPS)                 | 90 days      | 270 days       |
| Platinum | 500             | 0.1% (10 BPS)                 | 180 days     | 365 days       |

**How it works:**
- Decay only begins after the **grace period** since the user's last activity.
- Each additional inactive day applies the decay rate **compounding** until `max_decay_days` is reached.
- Higher-tier users enjoy slower decay and longer grace periods, rewarding consistent participation.

**Example:** A Bronze-tier user who has been inactive for 60 days (30 days past grace):
- Day 31: score * (1 - 0.005) = score * 0.995
- Day 32: (score * 0.995) * 0.995
- ... continues for up to 90 days of decay.

---

## Staleness Levels & Penalties

In addition to decay, reputation scores are adjusted by a **staleness penalty** multiplier based on the user's last activity:

| Staleness Level | Inactivity Range | Voting Weight Multiplier |
|-----------------|------------------|--------------------------|
| Active          | ≤ 30 days        | 100% (no penalty)        |
| Aging           | 31–90 days       | 80%                      |
| Stale           | 91–180 days      | 50%                      |
| Critical        | > 180 days       | 20%                      |

The staleness penalty is **applied on top of** the decay schedule. A user whose reputation has decayed to 70% of its original value and is in the "Stale" tier would see an effective reputation of:

```
effective_score = decayed_score * 0.50
```

---

## Impact on Voting Power

When `cast_reputation_weighted_vote` is called, the voting power is calculated as:

```rust
let multiplier = 10_000 + (reputation_score / 2);  // in BPS
let weighted_vote = token_power * multiplier / 10_000;
```

Where `reputation_score` already reflects both tier-based decay and staleness penalties. This means:
- A user with a **decayed reputation** gets a proportionally smaller voting multiplier.
- A user with a **stale reputation** (e.g., 50% weight) gets a further reduced effective score.
- Active, high-reputation users retain their full voting influence.

---

## Forcing a Stale-Score Refresh

Any user can trigger a **stale-score refresh** for any address via:

```
refresh_reputation(user: Address) -> u32
```

This re-evaluates the user's tier, recalculates decay, applies staleness penalties, and stores the updated score on-chain. This is useful when:
- A previously active user returns to governance and wants their score recalculated.
- A dApp or frontend wants to display the most up-to-date reputation.

---

## Admin Configuration

The admin can control decay and stale-penalty behaviour at a global level via:

```
update_reputation_config(admin: Address, config: ReputationConfig)
```

The `ReputationConfig` struct has three fields:
- `decay_enabled: bool` — Set to `false` to disable decay entirely.
- `stale_penalty_enabled: bool` — Set to `false` to disable staleness penalties.
- `default_tier: ReputationTier` — Default tier assigned to new users (default: Bronze).

---

## Summary

| Feature | Description |
|---------|-------------|
| **Tier-based decay** | Four tiers (Bronze/Silver/Gold/Platinum) with different decay rates and grace periods. |
| **Staleness detection** | Four staleness levels (Active/Aging/Stale/Critical) with progressive voting weight penalties. |
| **Fresh score in votes** | `cast_reputation_weighted_vote` uses the latest decayed/stale-adjusted score. |
| **Manual refresh** | Anyone can call `refresh_reputation` to force a recalculated score. |
| **Admin config** | Global toggle for decay and stale penalties; configurable default tier. |
