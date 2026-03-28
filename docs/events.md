 main
# Signal Registry Events

## SignalSubmitted
Emitted when a new signal is submitted via create_signal().

**Topics:** (`signal_registry`, `signal_submitted`)
**Data:** `{ signal_id: u64, provider: Address, asset_pair: String, action: SignalAction, expiry: u64, risk_rating: RiskLevel }`

## SignalExpired
Emitted when signal status changes to Expired during expiry check.

**Topics:** (`signal_registry`, `signal_expired`)
**Data:** `{ signal_id: u64, provider: Address, expired_at: u64 }`

## Other Events
- `trade_executed`
- `signal_status_changed`
- `provider_stats_updated`
- `follow_gained`
- `emergency_paused`
- etc. (see events.rs)
=======
# StellarSwipe contract events

This document lists **contract events** emitted via `env.events().publish(topics, data)` across the five workspace contracts under `stellar-swipe/contracts/`. It is the canonical reference for frontends and off-chain indexers.

## How Soroban events are shaped

Each emission has:

- **Topics**: first element is usually a `Symbol` event name; additional topics are often addresses, IDs, or other indexable keys.
- **Data**: a single `ScVal` payload (frequently a tuple or struct) carrying the main fields.

When decoding from RPC / Horizon-style responses, map XDR `ContractEvent` topics and body to the Rust types below. Short symbols are often created with `symbol_short!("…")` (max 9 chars); longer names use `Symbol::new(env, "…")`.

**Logical signature** in each subsection uses the form `name(field: Type, …)` combining topics (after the name) and data for readability.

**Example payload** values are realistic illustrations in JSON-like form; on-chain values are Stellar `ScVal` (addresses are `Address`, amounts often `i128` stroops or raw units as implemented).

---

## 1. `signal_registry` (`contracts/signal_registry`)

Defined in [`contracts/signal_registry/src/events.rs`](../stellar-swipe/contracts/signal_registry/src/events.rs) unless noted.

| Logical signature | Trigger (when) | Emitted from (call path) |
|-------------------|----------------|---------------------------|
| `admin_transferred(old_admin: Address, new_admin: Address)` | Admin transfers ownership | `admin::transfer_admin` |
| `parameter_updated(parameter: Symbol, old_value: i128, new_value: i128)` | Numeric admin parameter change | `admin::set_*` helpers |
| `trading_paused(paused_by: Address, timestamp: u64, expires_at: u64)` | Legacy/global trading pause | `events::emit_trading_paused` (if invoked) |
| `trading_unpaused(unpaused_by: Address, timestamp: u64)` | Trading resumes | `events::emit_trading_unpaused` (if invoked) |
| `multisig_signer_added(signer: Address, added_by: Address)` | Multisig signer added | `admin::add_multisig_signer` |
| `multisig_signer_removed(signer: Address, removed_by: Address)` | Multisig signer removed | `admin::remove_multisig_signer` |
| `fee_collected(asset_symbol: Symbol, provider: Address, platform_treasury: Address, total_fee: i128, platform_fee: i128, provider_fee: i128)` | Trade fee split recorded | `fees::collect_fee` |
| `signal_expired(provider: Address, signal_id: u64, expiry_time: u64)` | Signal passed expiry | `expiry::cleanup` paths |
| `trade_executed(signal_id: u64, executor: Address, roi: i128, volume: i128)` | Copy trade recorded | `SignalRegistry::execute_trade` |
| `signal_status_changed(signal_id: u64, provider: Address, old_status: u32, new_status: u32)` | `SignalStatus` discriminant changes | After execution paths in `lib.rs` |
| `provider_stats_updated(provider: Address, success_rate: u32, avg_return: i128, total_volume: i128)` | Provider aggregates updated | `lib.rs` after trade |
| `follow_gained(provider: Address, user: Address, new_count: u32)` | New follower | `social::follow_provider` |
| `follow_lost(provider: Address, user: Address, new_count: u32)` | Unfollow | `social::unfollow_provider` |
| `tags_added(signal_id: u64, provider: Address, tag_count: u32)` | Tags mutated on signal | `lib.rs` |
| `collab_signal_created(signal_id: u64, authors: Vec<Address>)` | Collaborative signal created | `lib.rs` collaboration flow |
| `collab_signal_approved(signal_id: u64, approver: Address)` | Co-author approved | `lib.rs` |
| `collab_signal_published(signal_id: u64)` | Collaborative signal goes live | `lib.rs` |
| `data_exported(requester: Address, entity_type: u32, record_count: u32)` | Export API used | `events::emit_data_exported` (if invoked) |
| `combo_created(combo_id: u64, provider: Address, component_count: u32)` | Combo signal created | `SignalRegistry::create_combo_signal` |
| `combo_executed(combo_id: u64, executor: Address, combined_roi: i128)` | Combo execution | `SignalRegistry::execute_combo_signal` |
| `combo_cancelled(combo_id: u64, provider: Address)` | Provider cancels combo | `SignalRegistry::cancel_combo_signal` |
| `signal_updated(signal_id: u64, updater: Address, version: u32)` | Signal version bump | `versioning::record_signal_update` |
| `copy_recorded(signal_id: u64, user: Address, version: u32)` | Copy tracking | `versioning::record_copy` |
| `cross_chain_requested(provider: Address, source_chain: String, source_id: String)` | Import request from L1/L2 id | `lib.rs` cross-chain |
| `cross_chain_imported(stellar_id: u64, source_chain: String, source_id: String)` | Mapped external signal stored | `lib.rs` |
| `cross_chain_address_registered(stellar_address: Address, source_chain: String, source_address: String)` | Address map entry | `lib.rs` |
| `cross_chain_synced(source_chain: String, source_id: String, new_status: u32)` | Status sync from source chain | `lib.rs` |
| `emergency_paused(category: String, paused_by: Address, reason: String, auto_unpause_at: Option<u64>)` | Category pause | `admin::pause_category` |
| `emergency_unpaused(category: String, unpaused_by: Address)` | Category unpause | `admin::unpause_category` |
| `circuit_breaker_triggered(category: String, reason: String)` | Circuit breaker trips | `admin` CB paths |

**Example — `fee_collected`**

```json
{
  "topics": ["fee_collected", "XLM", "GPROV…AAAA", "GTRES…AAAA"],
  "data": {
    "total_fee": 3500000,
    "platform_fee": 2450000,
    "provider_fee": 1050000
  }
}
```

**Example — `trade_executed`**

```json
{
  "topics": ["trade_executed", 42, "GEXEC…AAAA"],
  "data": { "roi": 1250000, "volume": 100000000 }
}
```

`SignalStatus` enum discriminants (for `signal_status_changed`): `Pending=0`, `Active=1`, `Executed=2`, `Expired=3`, `Successful=4`, `Failed=5` (see `types::SignalStatus`).

---

## 2. `oracle` (`contracts/oracle`)

### 2.1 Core oracle (`src/events.rs`, `src/lib.rs`)

| Logical signature | Trigger | Emitted from |
|-------------------|---------|--------------|
| `oracle_removed(oracle: Address, reason: String)` | Oracle removed from set | `emit_oracle_removed` |
| `weight_adjusted(oracle: Address, old_weight: u32, new_weight: u32, reputation: u32)` | Weight update after reputation | `emit_weight_adjusted` |
| `oracle_slashed(oracle: Address, reason: String, penalty: u32)` | Slashing applied | `emit_oracle_slashed` |
| `price_submit(oracle: Address, price: i128)` | Single oracle price posted | `emit_price_submitted` |
| `consensus_reached(price: i128, num_oracles: u32)` | Aggregated consensus price | `emit_consensus_reached` |
| `RECOVER(pair: AssetPair, timestamp: u64)` | Staleness auto-recovery clears pause flag on pair | `on_price_update` in `lib.rs` |
| `oracle_paused(timestamp: u64)` | Emergency pause proposal executes | `governance::exec_emergency_pause` (see §2.2) |

**Example — `consensus_reached`**

```json
{
  "topics": ["consensus", "reached"],
  "data": { "price": 123456000000, "num_oracles": 5 }
}
```

**Example — `RECOVER`**

```json
{
  "topics": ["RECOVER", "<AssetPair ScVal>"],
  "data": 1735689600
}
```

### 2.2 Oracle governance submodule (`src/governance.rs`)

These use two short topic symbols: `gov` + action. `ProposalType` is `AddOracle | RemoveOracle | UpdateParameter | EmergencyPause`.

| Topics | Data | Trigger |
|--------|------|---------|
| `gov`, `proposed` | `(id: u64, proposer: Address, proposal_type: ProposalType)` | `create_proposal` |
| `gov`, `vote` | `(proposal_id, voter: Address, vote: bool, weight: i128)` | `cast_vote` |
| `gov`, `executed` | `id: u64` | Successful execution |
| `gov`, `failed` | `(id: u64, reason: String)` | Execution or vote failure path |
| `gov`, `cancelled` | `id: u64` | Cancelled proposal |
| `gov`, `stake` | `(staker: Address, amount: i128, total: i128)` | Stake deposit/withdraw (`amount` negative on withdraw) |
| `gov`, `deposit` | `(recipient_or_proposer: Address, amount: i128, returned: bool)` | Deposit returned (`true`) or burned (`false`) |
| `oracle`, `paused` | `timestamp: u64` | Emergency pause execution |

**Example — `gov` + `vote`**

```json
{
  "topics": ["gov", "vote"],
  "data": {
    "proposal_id": 3,
    "voter": "GVOTER…AAAA",
    "vote": true,
    "weight": 500000000000
  }
}
```

---

## 3. `governance` (`contracts/governance` — DAO / token)

Symbols are mostly `symbol_short!` (≤9 characters). Voting power uses reputation-weighted paths where noted.

| Topics | Data | Trigger / Emitter |
|--------|------|-------------------|
| `gov`, `init` | `(admin, name: String, symbol: String, total_supply: i128)` | Token/governance `initialize` |
| `gov`, `dist` | `(team, early_investors, community_rewards, liquidity_mining, treasury, public_sale)` each `i128` | Distribution init |
| `gov`, `vestadd` | `(beneficiary, amount, cliff_seconds, duration_seconds)` as `i128` time fields | Vesting schedule |
| `gov`, `vestrel` | `(beneficiary, amount)` | Vesting release |
| `gov`, `stake` | `(holder, amount, is_stake: bool)` | Stake/unstake flag |
| `gov`, `accrue` | `(beneficiary, volume, reward)` | Liquidity reward accrual |
| `gov`, `claim` | `(beneficiary, amount)` | Reward claim |
| `gov`, `<action>` | `(actor: Address, value: i128)` | Admin treasury/committee actions; see table below |
| `gov`, `propnew` | `(id, proposer, voting_starts, voting_ends)` | New DAO proposal (`proposals.rs`) |
| `gov`, `repvote` | `(proposal_id, voter, token_power, weighted, multiplier)` | Reputation vote (`reputation.rs`) |
| `gov`, `badges` | `(user: Address, awarded: Vec<String>)` | Badges granted |
| `qv`, `alloc` | `(user: Address, capped: i128)` | Quadratic credit allocation |
| `qv`, `vote` | `(proposal_id, voter, votes_desired, credits_required)` | QV vote |
| `qv`, `verified` | `user: Address` | Identity verification flag |
| `qv`, `bonus` | `(user, bonus: i128)` | QV bonus |
| `qv`, `refund` | `(voter, proposal_id, credits_spent: i128)` | Refund on proposal end |
| `gov`, `cvote` | `(pool_id, proposal_id, voter, tokens_to_commit)` | Conviction voting |

**`gov` + dynamic `action` symbols** (`lib.rs` `emit_admin_action`): `votelock`, `rewardcfg`, `trsasset`, `budget`, `spend`, `recur`, `payrun`, `cmtadd`, `cmtelect`, `cmtfinal`, `cmtrank`, `cmtover`, `cmtdrop`, `target`, `rebalance`.

**Example — `gov` + `claim`**

```json
{
  "topics": ["gov", "claim"],
  "data": {
    "beneficiary": "GBEN…AAAA",
    "amount": 25000000000
  }
}
```

---

## 4. `bridge` (`contracts/bridge`)

String topic names via `Symbol::new(env, "…")`.

### 4.1 Monitoring & transfers (`src/monitoring.rs`)

| Event name (topic[0]) | Topics | Data |
|-----------------------|--------|------|
| `transaction_monitoring_started` | `transfer_id` | `(source_chain: u32, tx_hash: Bytes)` |
| `transaction_finalized` | `transfer_id` | `confirmations: u32` |
| `transfer_reset_reorg` | `transfer_id` | `timestamp: u64` |
| `reorg_handled` | `transfer_id` | `confirmations: u32` |
| `reorg_detected` | `transfer_id` | `(old_block: u64, new_block: u64)` |
| `monitoring_failed` | `transfer_id` | `timestamp: u64` |
| `bridge_transfer_created` | `transfer_id` | `(source_chain: u32, destination_chain: u32)` |
| `validator_signature_added` | `transfer_id` | `signature_count: u32` |
| `transfer_approved_minting` | `transfer_id` | `timestamp: u64` |
| `transfer_complete` | `transfer_id` | `timestamp: u64` |

### 4.2 Messaging (`src/messaging.rs`)

| Event | Topics | Data |
|-------|--------|------|
| `msg_sent` | `id` | `(target_chain: u32, sender: Address)` |
| `msg_relayed` | `message_id` | `validator: Address` |
| `msg_delivered` | `message_id` | `delivered_at: u64` |
| `callback_received` | `original_message_id` | `(sender: Address, callback_payload: Bytes)` |
| `msg_failed` | `message_id` | `timestamp: u64` |
| `msg_retry` | `message_id` | `timestamp: u64` |
| `msg_expired` | `message_id` | `timestamp: u64` |

### 4.3 Governance (`src/governance.rs`)

| Event | Topics | Data |
|-------|--------|------|
| `governance_initialized` | `bridge_id` | `required_signatures: u32` |
| `bridge_initialized` | `bridge_id` | `min_validator_signatures: u32` |
| `proposal_created` | `bridge_id`, `proposal_id` | `proposer: Address` |
| `proposal_signed` | `bridge_id`, `proposal_id` | `(signer: Address, signature_count: u32)` |
| `proposal_executed` | `bridge_id`, `proposal_id` | `timestamp: u64` |
| `validator_added` / `validator_removed` | `bridge_id` | `validator: Address` |
| `security_limits_updated` | `bridge_id` | `max_transfer_amount: i128` |
| `bridge_paused` / `bridge_unpaused` | `bridge_id` | `timestamp: u64` |
| `required_signatures_updated` | `bridge_id` | `new_count: u32` |
| `emergency_withdraw` | `bridge_id` | `(asset_id: Address, amount: i128, recipient: Address)` |
| `emergency_executed` | `bridge_id`, `proposal_id` | `timestamp: u64` |
| `proposal_cancelled` | `bridge_id`, `proposal_id` | `caller: Address` |
| `signer_added` / `signer_removed` | `bridge_id` | `Address` |

### 4.4 Fees (`src/fees.rs`)

| Event | Topics | Data |
|-------|--------|------|
| `bridge_fee_collected` | `transfer_id` | `(user, fee, amount, net_amount)` |
| `validator_reward_dist` | `bridge_id` | `(validator, per_validator: i128)` |
| `treasury_allocation` | `bridge_id` | `treasury_share: i128` |
| `bridge_fees_adjusted` | `bridge_id` | `(base_fee_bps: u32, utilization: u32)` |
| `bridge_fee_refunded` | `transfer_id` | `(user, fee_paid: i128, reason: String)` |

**Example — `bridge_transfer_created`**

```json
{
  "topics": ["bridge_transfer_created", 1001],
  "data": { "source_chain": 1, "destination_chain": 2 }
}
```

---

## 5. `auto_trade` (`contracts/auto_trade`)

Events use a mix of `Symbol::new` and `symbol_short!`. Struct payloads (`Trade`, `AuthConfig`, `ArbitrageExecutedEvent`, `FillEvent`, `AutoSellResult`, `RiskConfig`, `StatArbPortfolio`, etc.) serialize as contract-defined composite types.

### 5.1 Core trading & risk (`lib.rs`)

| Topics | Data | When |
|--------|------|------|
| `stop_loss_triggered`, `user`, `asset_id` | `trigger_price: i128` | Risk engine triggers stop on execute path |
| `trade_executed`, `user`, `signal_id` | `Trade` struct | After order execution |
| `risk_limit_block`, `user`, `signal_id` | `requested_amount: i128` | Trade status failed (risk) |
| `risk_config_updated`, `user` | `RiskConfig` | User updates risk profile |
| `trailing_stop_triggered` or `stop_loss_triggered`, `user`, `asset_id` | `AutoSellResult` | `process_price_update` advanced exit |
| `corr_limit_breach`, `user`, `new_asset` | `new_amount: i128` | Correlation guard rejects trade |

### 5.2 Auth & rate limits (`auth.rs`, `rate_limit.rs`)

| Topics | Data |
|--------|------|
| `auth_granted`, `user` | `AuthConfig` |
| `auth_revoked`, `user` | `()` |
| `user_whitelisted`, `user` | `()` |
| `rate_limit_violation`, `user` | `(violation_type: ViolationType, penalty_duration: u64, violation_count: u32)` |
| `rl_adjust` | `(load_pct: u32, per_user_hourly_transfers: u32)` |

### 5.3 Emergency / bridge pause (`emergency.rs`)

| Topics | Data |
|--------|------|
| `pause_attempt_ignored`, `caller` | `reason: String` |
| `bridge_paused`, `caller`, `pause_type` | `reason: String` |
| `bridge_auto_unpaused` | `timestamp: u64` |
| `auto_unpause_scheduled` | `unpause_at: u64` |
| `recovery_initiated`, `caller` | `recovery_id: u64` |
| `recovery_checklist_complete` | `recovery_id: u64` |
| `bridge_unpaused`, `caller` | `(recovery_id: u64, pause_duration: u64)` |

### 5.4 Conditional orders (`conditional.rs`)

`cond_order_created`, `cond_order_cancelled`, `cond_order_expired`, `cond_order_triggered`, `cond_order_executed` — topics include `user` and `order_id` where applicable; data carries asset/amount or `()`.

### 5.5 TWAP (`twap.rs`)

| Event | Topics | Data |
|-------|--------|------|
| `TWAPOrderCreated` | `user`, `order_id` | `(total_amount, duration_minutes, segments)` |
| `TWAPSegmentExecuted` | `order_id`, `segment_index` | `(fill_amount, price)` |
| `TWAPSegmentFailed` | `order_id` | `segments_executed: u32` |
| `TWAPOrderComplete` | `order_id` | `(filled_amount, avg_price)` |
| `TWAPAdjusted` | `order_id` | `(reason: String, new_interval_seconds: u64)` |
| `TWAPOrderCancelled` | `order_id` | `(filled_amount, remaining_amount)` |

### 5.6 Strategies

- **DCA** (`dca.rs`): `dca_created`, `dca_purchase`, `dca_failed`, `dca_missed`, `dca_updated`, `dca_paused_funds`, `dca_paused`, `dca_resumed`.
- **Mean reversion** (`mean_reversion.rs`): `mr_strategy_created`, `mr_trade_opened`, `mr_position_closed`, `mr_params_adjusted`.
- **Grid** (`grid.rs`): `grid_init`, `grid_placed`, `grid_profit`, `grid_rebalance`, `grid_adjusted`.
- **Pairs** (`pairs_trading.rs`): `pairs_trade_exec`, `pairs_pos_closed`.
- **Stat arb** (`stat_arb.rs`): `stat_arb_configured`, `stat_arb_opened`, `stat_arb_rebalanced`, `stat_arb_closed` (data: `StatArbPortfolio` or `StatArbExitReason`).
- **Sentiment** (`sentiment.rs`): `sentiment_strategy_created`, `sentiment_trade_executed`, `sentiment_position_closed`.
- **Arbitrage** (`arbitrage.rs`): `arbitrage_executed`, `user` topic + `ArbitrageExecutedEvent` data.

### 5.7 Other modules

| Area | Events |
|------|--------|
| `iceberg.rs` | `iceberg_created`, `iceberg_filled`, `iceberg_replenished`, `iceberg_complete`, `iceberg_cancelled` |
| `referral.rs` | `referral_registered`, `referral_reward_earned` |
| `portfolio_insurance.rs` | `hedge_applied`, `hedges_rebalanced`, `hedges_removed` |
| `risk_parity.rs` | `risk_parity_rebalance`, `user` + `timestamp` |

**Example — `trade_executed`**

```json
{
  "topics": ["trade_executed", "GUSER…AAAA", 7],
  "data": {
    "signal_id": 7,
    "user": "GUSER…AAAA",
    "requested_amount": 50000000,
    "executed_amount": 50000000,
    "executed_price": 11234500,
    "timestamp": 1735690000,
    "status": "Filled"
  }
}
```

---

## Frontend / indexer integration checklist

Use this list to confirm the doc is sufficient:

1. **Subscribe** to contract IDs for each deployed wasm (per environment).
2. **Filter** by topic[0] `Symbol` (string) matching the names above; note `symbol_short!` truncates to ≤9 chars (`gov`, `vote`, `dist`, …).
3. **Decode** data using the contract’s published JSON spec / client bindings (`stellar contract bindings` or equivalent) so struct fields match on-chain order.
4. **Cross-check** this file whenever a PR touches `publish(` — see the PR template checkbox for `docs/events.md`.

---

## Maintenance

Any PR that adds or changes event emissions must update this file in the same PR (see [`.github/pull_request_template.md`](../.github/pull_request_template.md)).
 main
