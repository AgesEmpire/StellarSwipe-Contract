# Cross-Contract Call Graph — Auth Annotations

All `invoke_contract` / `try_invoke_contract` calls across the five StellarSwipe contracts,
with their auth requirements and verification status.

## Call Graph

```
TradeExecutorContract
  ├── execute_copy_trade(user, ...)
  │     ├── [auth] user.require_auth() ← BEFORE invoke          ✅
  │     ├── risk_gates::check_position_limit
  │     │     └── portfolio.get_open_position_count(user)        ✅ read-only, no auth needed
  │     └── portfolio.record_copy_position(user)                 ✅ user auth covers sub-invocation
  │
  ├── cancel_copy_trade(caller, user, trade_id, ...)
  │     ├── [auth] caller.require_auth() + assert caller==user   ✅
  │     ├── portfolio.has_position(user, trade_id)               ✅ read-only, no auth needed
  │     ├── sdex_router.swap(...)                                ✅ contract is spender (approve first)
  │     └── portfolio.close_position_by_executor(executor, ...) ✅ executor.require_auth() in callee
  │           └── executor == env.current_contract_address()
  │
  ├── check_and_trigger_stop_loss(user, trade_id, asset_pair)
  │     ├── oracle.get_price(asset_pair)                         ✅ read-only, no auth needed
  │     └── portfolio.close_position_by_executor(executor, ...) ✅ executor.require_auth() in callee
  │
  └── check_and_trigger_take_profit(user, trade_id, asset_pair)
        ├── oracle.get_price(asset_pair)                         ✅ read-only, no auth needed
        └── portfolio.close_position_by_executor(executor, ...) ✅ executor.require_auth() in callee

UserPortfolio
  └── get_pnl(user)
        └── oracle.get_price()  [try_invoke_contract]           ✅ read-only, graceful failure

FeeCollector
  ├── collect_fee(trader, token, amount, asset)
  │     ├── [auth] trader.require_auth() ← BEFORE invoke        ✅
  │     ├── token.transfer(trader → contract, fee_amount)        ✅ trader auth covers transfer
  │     └── oracle.convert_to_base(amount, asset) [try_invoke]  ✅ read-only, no auth needed
  │
  ├── claim_fees(provider, token)
  │     ├── [auth] provider.require_auth() ← BEFORE invoke      ✅
  │     └── token.transfer(contract → provider, amount)          ✅ contract is sender
  │
  └── withdraw_treasury_fees(recipient, token, amount)
        ├── [auth] admin.require_auth() ← BEFORE invoke         ✅
        └── token.transfer(contract → recipient, amount)         ✅ contract is sender
```

## Auth Model for `close_position_by_executor`

Stop-loss and cancel-copy-trade flows are **keeper-callable** — no user signature is available
at trigger time. The `UserPortfolio` exposes two close entrypoints:

| Entrypoint | Who calls it | Auth required |
|---|---|---|
| `close_position(user, id, pnl)` | User directly | `user.require_auth()` |
| `close_position_by_executor(executor, user, id, pnl)` | TradeExecutorContract | `executor.require_auth()` + `executor == stored AuthorizedExecutor` |

The `AuthorizedExecutor` address is set by the `UserPortfolio` admin via
`set_authorized_executor(executor)`. Only the registered executor contract may call
`close_position_by_executor`; any other address panics.

## Issues Fixed

| # | Location | Issue | Fix |
|---|---|---|---|
| A | `trade_executor/src/lib.rs` `cancel_copy_trade` | Called `portfolio.close_position(user,...)` — `user.require_auth()` inside callee would fail because user auth is not propagated as a sub-invocation | Changed to `close_position_by_executor(executor, user, ...)` |
| B | `trade_executor/src/triggers.rs` stop-loss / take-profit | Called `portfolio.close_position(user,...)` with no user auth at all (keeper-triggered) | Changed to `close_position_by_executor(executor, user, ...)` |

## Calls Verified Correct (No Changes Needed)

| Call | Reason |
|---|---|
| `portfolio.get_open_position_count(user)` | Read-only; no auth needed on callee |
| `portfolio.has_position(user, trade_id)` | Read-only; no auth needed on callee |
| `portfolio.record_copy_position(user)` | `user.require_auth()` called before invoke; user auth covers sub-invocation |
| `oracle.get_price(asset_pair)` | Read-only oracle query |
| `oracle.convert_to_base(amount, asset)` | Read-only oracle query |
| `oracle.get_price()` (UserPortfolio) | `try_invoke_contract`, read-only |
| `router.swap(...)` | Contract approves router via `token.approve()` first; contract is the spender |
| `token.transfer(trader → contract, ...)` | `trader.require_auth()` called before |
| `token.transfer(contract → recipient, ...)` | Contract is the sender; no user auth needed |
