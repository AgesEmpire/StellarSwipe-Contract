# StellarSwipe Contract Events

Canonical reference for all events emitted by the five core StellarSwipe contracts.
All event structs are defined in `contracts/common/src/events.rs`.

## Format

Every event uses a **two-topic tuple** and a **typed struct body**:

```
topics: (contract_name: Symbol, event_name: Symbol)
data:   EventStruct { field: Type, ... }
```

Field names are **stable across contract versions**. Adding fields is allowed; renaming or removing is a breaking change.

---

## fee_collector

### `fee_rate_updated`
Emitted by `set_fee_rate`.

| Topic 0 | Topic 1 |
|---|---|
| `fee_collector` | `fee_rate_updated` |

**Body:** `EvtFeeRateUpdated`
| Field | Type |
|---|---|
| `old_rate` | `u32` |
| `new_rate` | `u32` |
| `updated_by` | `Address` |

---

### `withdrawal_queued`
Emitted by `queue_withdrawal`.

| Topic 0 | Topic 1 |
|---|---|
| `fee_collector` | `withdrawal_queued` |

**Body:** `EvtWithdrawalQueued`
| Field | Type |
|---|---|
| `recipient` | `Address` |
| `token` | `Address` |
| `amount` | `i128` |
| `available_at` | `u64` |

---

### `treasury_withdrawal`
Emitted by `withdraw_treasury_fees`.

| Topic 0 | Topic 1 |
|---|---|
| `fee_collector` | `treasury_withdrawal` |

**Body:** `EvtTreasuryWithdrawal`
| Field | Type |
|---|---|
| `recipient` | `Address` |
| `token` | `Address` |
| `amount` | `i128` |
| `remaining_balance` | `i128` |

---

### `fees_claimed`
Emitted by `claim_fees`.

| Topic 0 | Topic 1 |
|---|---|
| `fee_collector` | `fees_claimed` |

**Body:** `EvtFeesClaimed`
| Field | Type |
|---|---|
| `provider` | `Address` |
| `token` | `Address` |
| `amount` | `i128` |

---

## user_portfolio

### `badge_awarded`
Emitted when a milestone badge is granted (first trade, 10 trades, 5-trade profit streak, top-10 leaderboard, early adopter).

| Topic 0 | Topic 1 |
|---|---|
| `user_portfolio` | `badge_awarded` |

**Body:** `EvtBadgeAwarded`
| Field | Type | Notes |
|---|---|---|
| `user` | `Address` | |
| `badge_type` | `u32` | 0=FirstTrade 1=TenTrades 2=ProfitableStreak5 3=Top10Leaderboard 4=EarlyAdopter |
| `awarded_at` | `u64` | ledger timestamp |

---

## trade_executor

### `stop_loss_triggered`
Emitted by `check_and_trigger_stop_loss` when price ≤ stop-loss threshold.

| Topic 0 | Topic 1 |
|---|---|
| `trade_executor` | `stop_loss_triggered` |

**Body:** `EvtStopLossTriggered`
| Field | Type |
|---|---|
| `user` | `Address` |
| `trade_id` | `u64` |
| `stop_loss_price` | `i128` |
| `current_price` | `i128` |

---

### `take_profit_triggered`
Emitted by `check_and_trigger_take_profit` when price ≥ take-profit threshold.

| Topic 0 | Topic 1 |
|---|---|
| `trade_executor` | `take_profit_triggered` |

**Body:** `EvtTakeProfitTriggered`
| Field | Type |
|---|---|
| `user` | `Address` |
| `trade_id` | `u64` |
| `take_profit_price` | `i128` |
| `current_price` | `i128` |

---

### `trade_cancelled`
Emitted by `cancel_copy_trade` after a successful SDEX exit.

| Topic 0 | Topic 1 |
|---|---|
| `trade_executor` | `trade_cancelled` |

**Body:** `EvtTradeCancelled`
| Field | Type |
|---|---|
| `user` | `Address` |
| `trade_id` | `u64` |
| `exit_price` | `i128` |
| `realized_pnl` | `i128` |

---

## oracle

### `oracle_removed`
| Topic 0 | Topic 1 |
|---|---|
| `oracle` | `oracle_removed` |

**Body:** `EvtOracleRemoved` — `{ oracle: Address }`

---

### `price_submitted`
| Topic 0 | Topic 1 |
|---|---|
| `oracle` | `price_submitted` |

**Body:** `EvtPriceSubmitted` — `{ oracle: Address, price: i128 }`

---

### `consensus_reached`
| Topic 0 | Topic 1 |
|---|---|
| `oracle` | `consensus_reached` |

**Body:** `EvtConsensusReached` — `{ price: i128, num_oracles: u32 }`

---

### `weight_adjusted`
| Topic 0 | Topic 1 |
|---|---|
| `oracle` | `weight_adjusted` |

**Body:** `EvtWeightAdjusted` — `{ oracle: Address, old_weight: u32, new_weight: u32, reputation: u32 }`

---

### `oracle_slashed`
| Topic 0 | Topic 1 |
|---|---|
| `oracle` | `oracle_slashed` |

**Body:** `EvtOracleSlashed` — `{ oracle: Address, penalty: u32 }`

---

## signal_registry

### `signal_adopted`
| Topic 0 | Topic 1 |
|---|---|
| `signal_registry` | `signal_adopted` |

**Body:** `EvtSignalAdopted` — `{ signal_id: u64, adopter: Address, new_count: u32 }`

---

### `signal_expired`
| Topic 0 | Topic 1 |
|---|---|
| `signal_registry` | `signal_expired` |

**Body:** `EvtSignalExpired` — `{ signal_id: u64, provider: Address, expired_at_ledger: u64 }`

---

### `trade_executed`
| Topic 0 | Topic 1 |
|---|---|
| `signal_registry` | `trade_executed` |

**Body:** `EvtTradeExecuted` — `{ signal_id: u64, executor: Address, roi: i128, volume: i128 }`

---

### `reputation_updated`
| Topic 0 | Topic 1 |
|---|---|
| `signal_registry` | `reputation_updated` |

**Body:** `EvtReputationUpdated` — `{ provider: Address, old_score: u32, new_score: u32 }`
