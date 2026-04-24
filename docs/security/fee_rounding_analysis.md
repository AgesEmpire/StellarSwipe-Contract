# Fee Rounding Analysis

## Summary

All fee calculations in `FeeCollector` use **floor (truncating) division**.
This is user-favorable: traders are never charged more than their exact pro-rata
fee. The sub-unit remainder stays with the trader and is not retained by the
contract, so no unwithdrawable dust accumulates in the treasury.

---

## Formula

```
fee = floor(trade_amount × fee_rate_bps / 10_000)
```

Implemented as `fee_amount_floor(trade_amount, fee_rate_bps)` in `lib.rs`.

---

## Rounding Decision

| Path                        | Direction | Rationale                                      |
|-----------------------------|-----------|------------------------------------------------|
| User-paid fee (`collect_fee`) | Floor ↓  | User-favorable; standard DeFi convention       |
| Rebate-tier fee (Silver/Gold) | Floor ↓  | Same formula, discounted rate, same direction  |
| Provider pending fees        | Exact     | No rounding — full `fee_amount` is stored      |
| Treasury withdrawal          | Exact     | Admin specifies exact amount; no rounding      |

---

## Dust Analysis

### Where dust could arise

Dust accumulates when a rounding remainder is retained by the contract rather
than returned to the payer.

### FeeCollector paths

**`collect_fee`**

```
fee_amount = floor(trade_amount × fee_rate_bps / 10_000)
```

The contract receives exactly `fee_amount` tokens via `token.transfer`.
The remainder `(trade_amount × fee_rate_bps) mod 10_000` is never transferred
to the contract — it stays in the trader's wallet. **No dust.**

**`claim_fees`**

The full `pending_fees` balance is paid out. No division occurs. **No dust.**

**`withdraw_treasury_fees`**

Admin specifies the exact withdrawal amount. No division occurs. **No dust.**

### Conclusion

No unwithdrawable dust can accumulate. The treasury balance is always the exact
sum of all `fee_amount` values collected, and the entire balance is withdrawable
via `queue_withdrawal` + `withdraw_treasury_fees`.

---

## Minimum Trade Size

At the default rate of 30 bps, the minimum trade amount that produces a non-zero
fee is:

```
min_trade = ceil(10_000 / fee_rate_bps) = ceil(10_000 / 30) = 334 stroops
```

Trades below this threshold are rejected with `FeeRoundedToZero`. This prevents
zero-fee trades from bypassing the fee mechanism.

| Fee rate (bps) | Min trade (stroops) |
|----------------|---------------------|
| 1              | 10,000              |
| 30 (default)   | 334                 |
| 100            | 100                 |

---

## Dust Accumulation Over 1 Million Trades

Worst case: every trade produces a remainder of `fee_rate_bps - 1` sub-units.

```
max_dust_per_trade = (fee_rate_bps - 1) / 10_000  <  1 stroop
```

Since the remainder never enters the contract, total dust in the contract after
any number of trades is **0**.

---

## Test Coverage

| Test | What it verifies |
|------|-----------------|
| `fee_floor_exact_division` | No rounding when divisible |
| `fee_floor_rounds_down_not_up` | Floor, not ceiling |
| `fee_floor_one_stroop_trade` | Sub-minimum trade → 0 |
| `fee_floor_minimum_nonzero_result` | Boundary at 334 stroops |
| `fee_floor_max_rate` | 100 bps rate |
| `fee_floor_large_amount_no_overflow` | No overflow for large amounts |
| `fee_floor_overflow_returns_none` | Overflow returns `None` |
| `no_dust_accumulation_over_many_trades` | Treasury = Σ floor(fees), no extra |
| `rebate_tier_fee_also_rounds_down` | Silver/Gold tiers also floor |
| `collect_fee_rejects_zero_fee_trade` | `FeeRoundedToZero` guard |
