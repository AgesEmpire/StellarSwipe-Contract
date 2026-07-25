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

---

## StakeVault (`stake_vault`)

Structs and emit helpers live in `contracts/stake_vault/src/events.rs`. Event-name
topics come from `contracts/shared/src/event_topics.rs` (issue #585).

### `tier_up` / `tier_dn`
Emitted when a provider's stake balance crosses a Bronze/Silver/Gold tier boundary.

| Field | Type | Description |
|---|---|---|
| `provider` | `Address` | Staker whose tier changed |
| `old_tier` | `u32` | Previous tier (0=none, 1=Bronze, 2=Silver, 3=Gold) |
| `new_tier` | `u32` | New tier |
| `stake_balance` | `i128` | Balance after the change |
| `upgraded` | `bool` | `true` if the tier increased |

### `mindur`
Emitted when admin updates the minimum stake duration lock (voting power eligibility).

| Field | Type | Description |
|---|---|---|
| `duration_secs` | `u64` | New minimum lock duration, in seconds |

### `blwmin`
Emitted the first time a provider's stake drops below the configured minimum.

| Field | Type | Description |
|---|---|---|
| `provider` | `Address` | Provider below minimum |
| `current_stake` | `i128` | Current stake balance |
| `minimum` | `i128` | Configured minimum stake |

### `wdcool` (issue #816)
Emitted when admin (or an executed multisig proposal) updates the large-withdrawal cooldown.

| Field | Type | Description |
|---|---|---|
| `cooldown_secs` | `u64` | New cooldown duration, in seconds (bounded to `[0, 2_592_000]`) |

### `wdreq`
Emitted when a staker initiates a time-locked large-withdrawal request.

| Field | Type | Description |
|---|---|---|
| `staker` | `Address` | Requesting staker |
| `balance` | `i128` | Balance at request time |
| `unlock_at` | `u64` | Timestamp at which the withdrawal becomes actionable |

### `flashln`
Emitted when a same-ledger stake+unstake pattern is detected and blocked.

| Field | Type | Description |
|---|---|---|
| `staker` | `Address` | Address that triggered the pattern |
| `balance` | `i128` | Balance at detection time |
| `ledger_seq` | `u32` | Ledger sequence number |

### `slashcfg` (issue #816)
Emitted when admin (or an executed multisig proposal) reconfigures slash tier percentages.
Tiers must satisfy `minor_bps <= major_bps <= critical_bps <= 10_000`.

| Field | Type | Description |
|---|---|---|
| `minor_bps` | `u32` | Minor-severity slash percentage (basis points) |
| `major_bps` | `u32` | Major-severity slash percentage |
| `critical_bps` | `u32` | Critical-severity slash percentage |

### `prtunstk`
Emitted on a partial unstake (withdrawal that leaves a remaining staked balance).

| Field | Type | Description |
|---|---|---|
| `staker` | `Address` | Staker |
| `amount` | `i128` | Amount withdrawn |
| `remaining` | `i128` | Balance remaining after withdrawal |

### `slashed`
Emitted when a provider's stake (own + delegated) is slashed.

| Field | Type | Description |
|---|---|---|
| `provider` | `Address` | Slashed provider |
| `severity` | `u32` | Severity tier (0=Minor, 1=Major, 2=Critical) |
| `slash_amount` | `i128` | Total amount slashed (own + delegated), minimum 1 stroop |
| `slash_id` | `u64` | Unique monotonic slash identifier |
| `reason` | `Symbol` | Caller-supplied reason code |

### `apwindow`
Emitted when admin updates the slash appeal window.

| Field | Type | Description |
|---|---|---|
| `window_secs` | `u64` | New appeal window, in seconds (0 disables appeals) |

### `appealed`
Emitted when a provider submits an appeal against a slash.

| Field | Type | Description |
|---|---|---|
| `appellant` | `Address` | Provider filing the appeal (must be the slashed provider) |
| `slash_id` | `u64` | Slash being appealed |
| `evidence_uri` | `String` | Off-chain evidence URI |

### `apresolv`
Emitted when admin resolves a pending appeal.

| Field | Type | Description |
|---|---|---|
| `slash_id` | `u64` | Slash being resolved |
| `uphold` | `bool` | `true` = slash stands (funds burned); `false` = reversed (funds restored) |
| `provider` | `Address` | The slashed provider |

### `delegate`
Emitted when a delegator stakes on behalf of a provider.

| Field | Type | Description |
|---|---|---|
| `delegator` | `Address` | Delegator |
| `provider` | `Address` | Provider receiving delegated stake |
| `amount` | `i128` | Amount delegated |

### `ustkqueu`
Emitted when a staker's unstake request is placed in the FIFO settlement queue.

| Field | Type | Description |
|---|---|---|
| `staker` | `Address` | Staker |
| `ticket` | `u64` | Assigned queue ticket |
| `queue_position` | `u64` | Zero-based position at enqueue time |

### `ustkproc`
Emitted per staker when `process_unstake_queue` successfully settles their request.

| Field | Type | Description |
|---|---|---|
| `staker` | `Address` | Staker |
| `ticket` | `u64` | Queue ticket processed |
| `amount` | `i128` | Amount withdrawn |

### `batchslh` (issue #815)
Emitted once per `batch_slash_stake` call, summarizing the run. Each successfully
slashed provider also still emits its own `slashed` event for per-provider audit trails.

| Field | Type | Description |
|---|---|---|
| `processed_count` | `u32` | Number of providers successfully slashed |
| `total_slashed` | `i128` | Sum of amounts slashed across the batch |

### `batchapl` (issue #815)
Emitted once per `batch_resolve_appeal` call, summarizing the run. Each resolved
appeal also still emits its own `apresolv` event.

| Field | Type | Description |
|---|---|---|
| `processed_count` | `u32` | Number of appeals successfully resolved |

### Emergency multi-sig unstake (issue #754): `emgcfg` / `emgreq` / `emgappr` / `emgexp` / `emgexec`

| Topic | Fields |
|---|---|
| `emgcfg` | `required: u32`, `penalty_bps: u32`, `timeout_secs: u64` |
| `emgreq` | `staker: Address` |
| `emgappr` | `staker: Address`, `signer: Address`, `approvals_count: u32` |
| `emgexp` | `staker: Address` |
| `emgexec` | `staker: Address`, `gross: i128`, `penalty: i128`, `net: i128` |
| `amount` | `i128` | Amount released |
