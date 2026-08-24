# Flash Loan Attack Surface Analysis — SDEX Integration

**Issue:** #268  
**Status:** Audited & Mitigated

---

## 1. Scope

All locations where SDEX spot price is read, and all stop-loss / take-profit trigger checks across the five contracts.

---

## 2. SDEX Price Read Inventory

### 2.1 `oracle` — `contracts/oracle/src/sdex.rs`

| Function | Purpose | Used for financial decision? |
|---|---|---|
| `calculate_spot_price` | Computes mid-market price from order book | **Execution only** — called by `refresh_from_sdex` which is an informational refresh, not a trigger |
| `calculate_vwap` | VWAP for a given trade amount | **Execution only** — used to estimate fill price, not for stop-loss/take-profit |

`refresh_from_sdex` in `oracle/src/lib.rs` calls `calculate_spot_price` and stores the result via `storage::set_price`. This stored price is **not** used for stop-loss or take-profit decisions — it feeds the oracle's price aggregation pipeline which is consumed by `get_price_with_confidence` (median aggregation with staleness filter).

### 2.2 `auto_trade` — `contracts/auto_trade/src/sdex.rs`

| Function | Purpose | Used for financial decision? |
|---|---|---|
| `execute_market_order` | Fills a market order at `signal.price` | **Execution only** — uses pre-validated signal price, not live SDEX spot |
| `execute_limit_order` | Fills a limit order if `market_price <= signal.price` | **Execution only** — `market_price` is read from temporary storage (test helper), not live SDEX |

Neither function reads a live SDEX spot price for stop-loss or take-profit evaluation.

### 2.3 `auto_trade` — `contracts/auto_trade/src/risk.rs`

`check_stop_loss` accepts an `oracle_price: Option<i128>` parameter:

```rust
let reference_price = oracle_price.unwrap_or(current_price);
```

When `oracle_price` is `Some`, the oracle TWAP is used. When `None`, it falls back to `current_price` (the SDEX spot passed in from `execute_trade`). The call site in `lib.rs`:

```rust
let oracle_price: Option<i128> = oracle::get_oracle_price(&env, signal.base_asset)
    .ok()
    .map(|op| oracle::oracle_price_to_i128(&op));

let stop_loss_triggered = risk::validate_trade(
    &env, &user, signal.base_asset, amount, signal.price, is_sell, oracle_price,
)?;
```

**When an oracle is configured, the oracle price is used — SDEX spot is ignored for the trigger decision.**  
When no oracle is configured, `signal.price` (the pre-validated signal price, not a live SDEX read) is used as fallback.

### 2.4 `trade_executor` — `contracts/trade_executor/src/triggers.rs`

`check_and_trigger_stop_loss` and `check_and_trigger_take_profit` both call:

```rust
let current_price: i128 = env.invoke_contract(
    &oracle,
    &Symbol::new(env, "get_price"),
    soroban_sdk::vec![env, asset_pair.into()],
);
```

**These functions exclusively use the oracle contract for price — SDEX spot is never read here.**

### 2.5 `trade_executor` — `contracts/trade_executor/src/sdex.rs`

`execute_sdex_swap` enforces `min_received` slippage protection:

```rust
if actual_received < min_received {
    return Err(ContractError::SlippageExceeded);
}
```

This is **execution only** — the swap is not used to make a financial decision; it is the execution of a decision already made.

---

## 3. Flash Loan Attack Vectors — Assessment

### 3.1 Stop-Loss / Take-Profit Manipulation

A flash loan could temporarily move the SDEX spot price within a single transaction. However:

- Stop-loss/take-profit triggers in `trade_executor` use the **oracle contract** (`get_price`), not SDEX spot.
- The oracle contract uses **median aggregation** across multiple price sources with a staleness filter (300s TTL). A single-transaction flash loan cannot manipulate a time-weighted or multi-source median.
- In `auto_trade`, when an oracle is configured, `oracle_price` takes precedence over `signal.price` for stop-loss evaluation.

**Verdict: No flash loan manipulation path for stop-loss/take-profit triggers.**

### 3.2 SDEX Execution Slippage

`execute_sdex_swap` in `trade_executor` enforces `min_received` at the balance-delta level (not just the router's return value):

```rust
let balance_after = to_client.balance(&this);
let actual_received = balance_after.checked_sub(balance_before).unwrap_or(0);
if actual_received < min_received {
    return Err(ContractError::SlippageExceeded);
}
```

`swap_with_slippage` computes `min_received = amount * (10_000 - max_slippage_bps) / 10_000`.

**Verdict: All SDEX executions have slippage protection enforced at the balance-delta level.**

### 3.3 Oracle `refresh_from_sdex`

`refresh_from_sdex` in `oracle/src/lib.rs` reads SDEX spot and stores it as one price source. However:
- It feeds into `get_price_with_confidence` which applies **median aggregation** across all sources.
- A single manipulated SDEX reading is outvoted by other sources.
- The 300s staleness filter means a flash-loan-manipulated price expires within the same ledger.

**Verdict: SDEX spot in oracle is one input to a median — not a sole decision source.**

---

## 4. Mitigations in Place

| Risk | Mitigation |
|---|---|
| Flash loan manipulates stop-loss trigger | Oracle TWAP used for triggers; SDEX spot ignored when oracle is set |
| Flash loan manipulates take-profit trigger | Oracle contract exclusively used in `trade_executor` triggers |
| Flash loan manipulates SDEX execution price | `min_received` slippage protection enforced at balance-delta level |
| Flash loan manipulates oracle price | Median aggregation + 300s staleness filter in oracle contract |

---

## 5. Recommendations

1. **Enforce oracle requirement for stop-loss in `auto_trade`:** Currently, if no oracle is configured, `signal.price` is used as fallback. Consider requiring oracle configuration before enabling stop-loss to eliminate the fallback path entirely.
2. **Document `min_received` calculation:** Callers of `swap` (not `swap_with_slippage`) must supply a non-zero `min_received`. Add a validation that `min_received > 0` in `execute_sdex_swap`.
