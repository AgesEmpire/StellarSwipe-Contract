# StellarSwipe Contract Events

All events use a **two-topic format**:

```
topics[0]  contract_name : Symbol   (e.g. "fee_collector")
topics[1]  event_name    : Symbol   (e.g. "fee_collected")
body       <EventStruct>            (a #[contracttype] struct)
```

Event structs are defined in `contracts/shared/src/events.rs`.

## Event Versioning Policy

Every event struct carries a `schema_version: u32` field, starting at `1`.

- **Backward-compatible additions** (new optional fields, new events): keep the same `schema_version`.
- **Breaking changes** (field removal, type change, field rename): bump `schema_version` by 1 and document the change below.

Indexers MUST check `schema_version` before deserialising event bodies to handle multiple schema generations gracefully.

> **PR requirement:** Any PR that makes a breaking change to an event struct MUST bump `schema_version` and add an entry to the changelog table below.

### Version changelog

| Event | Version | Change |
|---|---|---|
| All events | 1 | Initial versioned schema — `schema_version` field added |

**Stability policy:** field names and types are stable across contract versions.
Adding new fields is allowed; removing or renaming fields requires a new event name.

---

## FeeCollector (`fee_collector`)

### `fee_collected`
Emitted when a trader pays a fee.

| Field | Type | Description |
|---|---|---|
| `trader` | `Address` | Trader who paid the fee |
| `token` | `Address` | Token used |
| `trade_amount` | `i128` | Notional trade amount |
| `fee_amount` | `i128` | Fee charged (floor-rounded) |
| `fee_rate_bps` | `u32` | Effective rate in basis points |

### `fee_rate_updated`
Emitted when admin changes the fee rate.

| Field | Type | Description |
|---|---|---|
| `old_rate` | `u32` | Previous rate in bps |
| `new_rate` | `u32` | New rate in bps |
| `updated_by` | `Address` | Admin address |

### `fees_claimed`
Emitted when a provider claims pending fees.

| Field | Type | Description |
|---|---|---|
| `provider` | `Address` | Provider claiming fees |
| `token` | `Address` | Token claimed |
| `amount` | `i128` | Amount claimed (0 if nothing pending) |

### `withdrawal_queued`
Emitted when admin queues a treasury withdrawal (starts timelock).

| Field | Type | Description |
|---|---|---|
| `recipient` | `Address` | Withdrawal destination |
| `token` | `Address` | Token to withdraw |
| `amount` | `i128` | Amount queued |
| `available_at` | `u64` | Timestamp when withdrawal unlocks |

### `treasury_withdrawal`
Emitted when a queued withdrawal is executed.

| Field | Type | Description |
|---|---|---|
| `recipient` | `Address` | Withdrawal destination |
| `token` | `Address` | Token withdrawn |
| `amount` | `i128` | Amount withdrawn |
| `remaining_balance` | `i128` | Treasury balance after withdrawal |

---

## TradeExecutor (`trade_executor`)

### `trade_cancelled`
Emitted when a user manually cancels a copy trade.

| Field | Type | Description |
|---|---|---|
| `user` | `Address` | Position owner |
| `trade_id` | `u64` | Trade identifier |
| `exit_price` | `i128` | SDEX swap output |
| `realized_pnl` | `i128` | `exit_price - entry_amount` |

### `stop_loss_triggered`
Emitted when a keeper triggers a stop-loss close.

| Field | Type | Description |
|---|---|---|
| `user` | `Address` | Position owner |
| `trade_id` | `u64` | Trade identifier |
| `stop_loss_price` | `i128` | Configured threshold |
| `current_price` | `i128` | Oracle price at trigger time |

### `take_profit_triggered`
Emitted when a keeper triggers a take-profit close.

| Field | Type | Description |
|---|---|---|
| `user` | `Address` | Position owner |
| `trade_id` | `u64` | Trade identifier |
| `take_profit_price` | `i128` | Configured threshold |
| `current_price` | `i128` | Oracle price at trigger time |

---

## UserPortfolio (`user_portfolio`)

### `trade_shareable`
Emitted on profitable position close (`realized_pnl > 0`). Used by the frontend to generate share cards.

| Field | Type | Description |
|---|---|---|
| `user` | `Address` | Position owner |
| `position_id` | `u64` | Position identifier |
| `asset_pair` | `u32` | Asset pair code |
| `entry_price` | `i128` | Entry price |
| `exit_price` | `i128` | Exit price |
| `pnl_bps` | `i64` | P&L in basis points |
| `signal_provider` | `Address` | Signal provider address |
| `signal_id` | `u64` | Signal identifier |

### `keeper_close`
Emitted when a keeper (TradeExecutor) closes a position via stop-loss or take-profit.

| Field | Type | Description |
|---|---|---|
| `user` | `Address` | Position owner |
| `position_id` | `u64` | Position identifier |
| `asset_pair` | `u32` | Asset pair code |

### `subscription_created`
Emitted when a user subscribes to a provider's premium feed.

| Field | Type | Description |
|---|---|---|
| `user` | `Address` | Subscriber |
| `provider` | `Address` | Signal provider |
| `expires_at` | `u64` | Subscription expiry timestamp |

---

## SignalRegistry (`signal_registry`)

### `signal_adopted`
Emitted when a signal's adoption count is incremented.

| Field | Type | Description |
|---|---|---|
| `signal_id` | `u64` | Signal identifier |
| `adopter` | `Address` | Address that adopted |
| `new_count` | `u32` | Updated adoption count |

### `signal_edited`
Emitted when a provider edits a signal within the 60-second edit window.

| Field | Type | Description |
|---|---|---|
| `signal_id` | `u64` | Signal identifier |
| `provider` | `Address` | Signal owner |
| `price` | `i128` | Updated price |
| `rationale_hash` | `String` | Updated rationale hash |
| `confidence` | `u32` | Updated confidence (0–100) |

### `reputation_updated`
Emitted when a provider's reputation score changes after a signal outcome.

| Field | Type | Description |
|---|---|---|
| `provider` | `Address` | Provider address |
| `old_score` | `u32` | Previous score |
| `new_score` | `u32` | Updated score |

---

## Governance (`governance`)

### `stake_changed`
Emitted when a holder stakes or unstakes tokens.

| Field | Type | Description |
|---|---|---|
| `holder` | `Address` | Token holder |
| `amount` | `i128` | Amount staked/unstaked |
| `is_stake` | `bool` | `true` = stake, `false` = unstake |

### `reward_claimed`
Emitted when a beneficiary claims liquidity mining rewards.

| Field | Type | Description |
|---|---|---|
| `beneficiary` | `Address` | Claimant |
| `amount` | `i128` | Amount claimed |

### `vesting_released`
Emitted when vested tokens are released to a beneficiary.

| Field | Type | Description |
|---|---|---|
| `beneficiary` | `Address` | Vesting recipient |
| `amount` | `i128` | Amount released |
