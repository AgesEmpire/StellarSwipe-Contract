# Fee Calculation Methodology

This document describes the dynamic fee optimization system implemented in
`fee_collector/src/fee_optimization.rs` (Issue #512).

---

## Fee Formula

```
effective_fee_bps = clamp(
    base_fee_bps + congestion_surcharge_bps - tier_discount_bps - volume_discount_bps,
    floor_fee_bps,
    ceiling_fee_bps
)
```

All values are expressed in **basis points** (1 bps = 0.01 %).

---

## Network Condition Monitoring

Network conditions are pushed by an off-chain monitor keeper via
`FeeCollectorContract::push_network_conditions`.  The snapshot includes:

- `load_pct` – percentage of theoretical throughput in use (0–100).
- `median_close_time_ms` – median ledger close time for the last 10 ledgers.
- `observed_at_ledger` – ledger sequence at which conditions were observed.

Snapshots older than `max_condition_age_ledgers` (default: 100) are considered
stale and produce no congestion surcharge.

---

## Congestion Surcharge

| Network load         | Surcharge |
|----------------------|-----------|
| 0 – 50 %             | 0 bps     |
| 51 – 80 %            | +10 bps   |
| > 80 %               | +40 bps   |

These defaults are configurable via `FeeOptimizerConfig`.

---

## Fee Tier Calculation

A user's fee tier is determined by their 30-day rolling volume:

| Tier     | 30-day volume          | Discount |
|----------|------------------------|----------|
| Standard | < 1 000 USD equivalent | 0 bps    |
| Silver   | 1 000 – 50 000         | -3 bps   |
| Gold     | 50 000 – 500 000       | -8 bps   |
| Platinum | > 500 000              | -15 bps  |

The tier must be passed explicitly by the caller; the contract does not
compute it automatically to avoid expensive cross-contract calls on every trade.

---

## Volume Discount

Epoch (e.g. daily or weekly) aggregate volume unlocks additional discounts:

| Epoch volume          | Discount |
|-----------------------|----------|
| < 1 000 000 units     | 0 bps    |
| 1 000 000 – 10 000 000| -2 bps   |
| > 10 000 000          | -5 bps   |

---

## Fee Predictability Mechanisms

Before presenting a "you will pay" amount, the front-end calls:

```typescript
const breakdown = await client.predict_fee({ tier: "gold", trade_volume: 500 });
console.log(`Effective fee: ${breakdown.effective_bps / 100}%`);
```

`predict_fee` is a read-only call; it never mutates state.

---

## Fee Transparency

Every `calculate_fee` call emits a `fee_opt` event with the full breakdown:

```
(effective_bps, congestion_surcharge_bps, tier_discount_bps, volume_discount_bps, load_pct)
```

Off-chain indexers can audit the fee structure by aggregating these events.

---

## Configuration

Administrators can update the optimizer via `set_fee_optimizer_config`:

| Field                      | Default | Description                        |
|----------------------------|---------|------------------------------------|
| `base_fee_bps`             | 30      | Base fee before adjustments        |
| `floor_fee_bps`            | 5       | Minimum fee regardless of discounts|
| `ceiling_fee_bps`          | 300     | Maximum fee regardless of surcharge|
| `congestion_med_surcharge_bps` | 10  | Surcharge at 51–80 % load          |
| `congestion_high_surcharge_bps`| 40  | Surcharge at > 80 % load           |
| `silver_discount_bps`      | 3       | Discount for Silver tier           |
| `gold_discount_bps`        | 8       | Discount for Gold tier             |
| `platinum_discount_bps`    | 15      | Discount for Platinum tier         |
| `volume_tier1_threshold`   | 1 000 000| Epoch volume for tier-1 discount  |
| `volume_tier1_discount_bps`| 2       | Discount at tier-1 epoch volume    |
| `volume_tier2_threshold`   | 10 000 000| Epoch volume for tier-2 discount |
| `volume_tier2_discount_bps`| 5       | Discount at tier-2 epoch volume    |
| `max_condition_age_ledgers`| 100     | Staleness limit for network data   |

Base fee is capped at `MAX_BASE_FEE_BPS = 500` to prevent admin misconfiguration.
