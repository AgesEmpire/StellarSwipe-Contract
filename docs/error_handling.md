# Error Handling and Recovery Patterns

This document describes the standardized error handling architecture
implemented across StellarSwipe contracts (Issue #511).

---

## Error Category Hierarchy

Every `AutoTradeError` and `ContractError` variant is classified into one of
five top-level categories defined in `auto_trade/src/error_handling.rs`:

```
ErrorCategory
├── Transient        – temporary; automatic retry is safe
│   ├── OracleUnavailable
│   ├── InsufficientLiquidity
│   └── SystemError
├── Validation       – bad caller input; retry unchanged is useless
│   ├── InvalidAmount / InvalidConfig / InvalidPriceData
│   └── InvalidSlippageSetting
├── Authorization    – permission failure
│   ├── Unauthorized
│   └── RateLimited
├── State            – operation disallowed by current contract state
│   ├── TradingPaused
│   ├── PositionLimitExceeded
│   └── DailyTradeLimitExceeded
└── Permanent        – unrecoverable; no retry makes sense
    ├── SignalExpired / SignalNotFound
    ├── StopLossTriggered
    └── SlippageExceeded
```

Call `AutoTradeContract::categorize_error(env, err_code)` from off-chain
tooling to classify a numeric error code without parsing the XDR enum.

---

## Recovery Strategies

The `recovery_for(env, err)` function (or the combined `record_and_recover`)
maps each error to a typed `RecoveryStrategy`:

| Strategy                  | When used                                           |
|---------------------------|-----------------------------------------------------|
| `RetryWithBackoff`        | Transient errors (oracle, liquidity)               |
| `RetryAfter { delay }`    | State errors (paused, limits)                       |
| `SwitchToFallback`        | `InsufficientLiquidity` → try SDEX instead          |
| `Degrade { description }` | `OracleUnavailable` → continue without oracle check |
| `Abort`                   | Validation / Authorization / Permanent              |

---

## Automatic Retry Logic

`RetryState` is a value type that tracks attempt count and last-attempt
timestamp.  Store it in persistent storage keyed by operation id:

```rust
let mut retry: RetryState = storage::get_retry_state(&env, op_id)
    .unwrap_or_else(|| RetryState::new(3, 5));

if retry.should_retry(env.ledger().timestamp()) {
    retry.record_attempt(env.ledger().timestamp());
    storage::set_retry_state(&env, op_id, &retry);
    // ... attempt the operation
} else {
    return Err(AutoTradeError::SystemError); // exhausted
}
```

The back-off delay doubles each attempt (5 s → 10 s → 20 s), capped at 10 min.

---

## Error Metrics Tracking

All errors handled via `record_error(env, err)` are accumulated in
`ErrorMetrics` (instance storage, persistent across transactions):

```rust
ErrorMetrics {
    transient_count:    u64,
    validation_count:   u64,
    authorization_count: u64,
    state_count:        u64,
    permanent_count:    u64,
    total_count:        u64,
    last_error_at:      u64,  // ledger timestamp
}
```

Query via `AutoTradeContract::get_error_metrics(env)`.
Reset via `AutoTradeContract::reset_error_metrics(env, admin)` (admin only).

Each `record_error` call also emits an `error_recorded` event so off-chain
dashboards can track failure rates without polling storage.

---

## Graceful Degradation

When a critical dependency (e.g. oracle) is unavailable:

1. The contract enters degraded mode:
   ```rust
   error_handling::enter_degraded_mode(&env, String::from_str(&env, "oracle_price_check"));
   ```
2. Downstream logic checks `is_degraded(&env)` and skips the oracle check,
   falling back to the last-known SDEX price.
3. Once the oracle recovers, the admin calls:
   ```rust
   error_handling::exit_degraded_mode(&env);
   ```

Events emitted: `degraded` (on enter) and `restored` (on exit).

---

## Error Documentation

| Error                   | Category      | Recovery                   |
|-------------------------|---------------|----------------------------|
| `OracleUnavailable`     | Transient     | Degrade / RetryWithBackoff |
| `InsufficientLiquidity` | Transient     | SwitchToFallback           |
| `SystemError`           | Transient     | RetryWithBackoff           |
| `Unauthorized`          | Authorization | Abort                      |
| `RateLimited`           | Authorization | Abort                      |
| `TradingPaused`         | State         | RetryAfter 60 s            |
| `PositionLimitExceeded` | State         | RetryAfter 60 s            |
| `DailyTradeLimitExceeded`| State        | RetryAfter 60 s            |
| `SignalExpired`         | Permanent     | Abort                      |
| `SlippageExceeded`      | Permanent     | Abort                      |
| `InvalidAmount`         | Validation    | Abort                      |

See `auto_trade/src/error_handling.rs` for the complete mapping.
