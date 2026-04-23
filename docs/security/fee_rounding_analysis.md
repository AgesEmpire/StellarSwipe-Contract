# Fee Rounding Analysis

## Summary

All fee arithmetic in `FeeCollector` is centralised in two helper functions in
`contracts/fee_collector/src/lib.rs`:

| Helper | Formula | Direction | Used for |
|---|---|---|---|
| `fee_amount_floor(amount, rate_bps)` | `amount * rate_bps / 10_000` (truncate) | Round **down** | User-paid fees |
| `fee_amount_ceil(amount, rate_bps)` | `ceil(amount * rate_bps / 10_000)` | Round **up** | Provider reward splits (future) |

## Rounding Strategy

### User-paid fees — round DOWN (user-favorable)

```
fee = floor(trade_amount × fee_rate_bps / 10_000)
```

The user never pays more than their exact fractional share. The sub-unit remainder
(at most `fee_rate_bps / 10_000 < 0.01%` of the trade) stays with the user.

**Example** — `trade_amount = 9_999`, `fee_rate = 30 bps`:
```
9_999 × 30 = 299_970
299_970 / 10_000 = 29  (remainder 9_970 discarded — stays with user)
```

### Provider reward splits — round UP (protocol-favorable)

When the protocol distributes collected fees to signal providers, rounding UP
ensures the provider receives at least their exact share. Any sub-unit excess
is taken from the already-collected treasury balance, so no new tokens are
created and no dust accumulates.

## Dust Analysis

### Does dust accumulate in the contract?

**No.** Here is why:

1. `collect_fee` transfers exactly `fee_amount_floor(...)` tokens from the trader
   to the contract, then credits that same amount to `treasury_balance`.
   `treasury_balance == actual_token_balance` at all times (modulo pending
   provider claims, which are also exact integers).

2. The foregone sub-unit remainder never enters the contract — it stays in the
   trader's wallet. There is therefore no sub-unit amount trapped inside the
   contract that cannot be withdrawn.

3. `withdraw_treasury_fees` withdraws an admin-specified integer amount, so any
   integer balance is fully withdrawable.

### Worst-case protocol revenue loss per trade

At `fee_rate = 100 bps` (maximum), the maximum foregone amount per trade is:

```
max_dust_per_trade = (10_000 - 1) × 100 / 10_000 = 0.9999 units ≈ 1 unit
```

Over 1 million trades at maximum rate, the total foregone revenue is at most
**~1 million units** (e.g. ~0.1 XLM at 7 decimals). This is the accepted
cost of user-favorable rounding.

### Analytics rounding (`avg_fee_per_trade`)

`analytics.rs` computes `avg_fee_per_trade = total_fees / trade_count` using
integer truncation. This value is **read-only** — it is never transferred or
stored as a balance — so it cannot produce unwithdrawable dust.

## Fee Rate Bounds

| Constant | Value | Meaning |
|---|---|---|
| `MIN_FEE_RATE_BPS` | 1 | 0.01% |
| `DEFAULT_FEE_RATE_BPS` | 30 | 0.30% |
| `MAX_FEE_RATE_BPS` | 100 | 1.00% |

The minimum non-zero fee at `MIN_FEE_RATE_BPS = 1` requires
`trade_amount >= 10_000` units. Trades below this threshold return
`ContractError::FeeRoundedToZero` and are rejected.

## Invariants

1. `treasury_balance(token) == sum of all fee_amount_floor values collected − sum of all withdrawals`
2. `fee_amount_floor(amount, rate) <= fee_amount_ceil(amount, rate) <= fee_amount_floor(amount, rate) + 1`
3. No token amount ever enters the contract without being credited to `treasury_balance` or `pending_fees`.
