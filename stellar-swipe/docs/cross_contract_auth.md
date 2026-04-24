# Cross-Contract Call Graph — Auth Annotations

This document maps every cross-contract call in the StellarSwipe contracts, the
auth requirement at the call site, and the auth check performed by the callee.

---

## Contracts

| Short name       | Crate                  |
|------------------|------------------------|
| TradeExecutor    | `trade_executor`       |
| UserPortfolio    | `user_portfolio`       |
| FeeCollector     | `fee_collector`        |
| SignalRegistry   | `signal_registry`      |
| Oracle           | external / `oracle`    |
| SEP-41 Token     | Stellar Asset Contract |
| SDEX Router      | external aggregator    |

---

## Call Graph

```
TradeExecutor
├── execute_copy_trade(user, token, amount)
│   ├── [1] SEP-41 Token → balance(user)
│   │       Auth: read-only, no auth required ✓
│   └── [2] UserPortfolio → validate_and_record(user, max_positions)
│           Auth: user.require_auth() called BEFORE this call ✓
│           Callee: panics if cap exceeded (surfaced as PositionLimitReached)
│
├── cancel_copy_trade(caller, user, trade_id, ...)
│   ├── [3] UserPortfolio → has_position(user, trade_id)
│   │       Auth: read-only; caller.require_auth() + caller==user checked BEFORE ✓
│   ├── [4] SDEX Router → swap(pull_from, from_token, to_token, amount, min_out, recipient)
│   │       Auth: contract swaps its own tokens; SEP-41 approve() called BEFORE ✓
│   └── [5] UserPortfolio → close_position(user, trade_id, pnl)
│           Auth: caller.require_auth() + caller==user checked BEFORE ✓
│           Callee: user.require_auth() inside close_position ✓
│
├── check_and_trigger_stop_loss(user, trade_id, asset_pair)  [keeper-callable]
│   ├── [6] Oracle → get_price(asset_pair)
│   │       Auth: read-only, no auth required ✓
│   └── [7] UserPortfolio → close_position_keeper(caller, user, position_id, asset_pair)
│           Auth: NO user auth required (keeper pattern)
│           Caller: env.current_contract_address() (TradeExecutor)
│           Callee: caller.require_auth() + caller==registered_trade_executor ✓
│
├── check_and_trigger_take_profit(user, trade_id, asset_pair)  [keeper-callable]
│   ├── [8] Oracle → get_price(asset_pair)
│   │       Auth: read-only, no auth required ✓
│   └── [9] UserPortfolio → close_position_keeper(caller, user, position_id, asset_pair)
│           Auth: same as [7] ✓
│
└── swap / swap_with_slippage(from_token, to_token, amount, ...)
    └── [10] SDEX Router → swap(pull_from, from_token, to_token, amount, min_out, recipient)
            Auth: contract swaps its own tokens; SEP-41 approve() called BEFORE ✓

UserPortfolio
└── subscribe_to_provider(user, provider, duration_days)
    └── [11] SEP-41 Token → transfer(user, provider, total)
            Auth: user.require_auth() called BEFORE ✓

FeeCollector
├── collect_fee(trader, token, trade_amount, trade_asset)
│   └── [12] SEP-41 Token → transfer(trader, contract, fee_amount)
│           Auth: trader.require_auth() called BEFORE ✓
│
├── claim_fees(provider, token)
│   └── [13] SEP-41 Token → transfer(contract, provider, amount)
│           Auth: provider.require_auth() called BEFORE ✓
│
└── withdraw_treasury_fees(recipient, token, amount)
    └── [14] SEP-41 Token → transfer(contract, recipient, amount)
            Auth: admin.require_auth() + timelock check BEFORE ✓

SignalRegistry
└── get_signal_for_viewer(signal_id, viewer)
    └── [15] UserPortfolio → check_subscription(user, provider)
            Auth: read-only, no auth required ✓
```

---

## Auth Summary Table

| # | Caller Contract  | Callee Contract  | Function                    | Auth at call site                          | Auth in callee                              | Status |
|---|------------------|------------------|-----------------------------|--------------------------------------------|---------------------------------------------|--------|
| 1 | TradeExecutor    | SEP-41 Token     | `balance(user)`             | None (read-only)                           | None                                        | ✓      |
| 2 | TradeExecutor    | UserPortfolio    | `validate_and_record`       | `user.require_auth()` before call          | Panics on cap exceeded                      | ✓      |
| 3 | TradeExecutor    | UserPortfolio    | `has_position`              | `caller.require_auth()` + `caller==user`   | None (read-only)                            | ✓      |
| 4 | TradeExecutor    | SDEX Router      | `swap` (cancel path)        | `caller.require_auth()` + `caller==user`   | SEP-41 `approve` pre-authorises pull        | ✓      |
| 5 | TradeExecutor    | UserPortfolio    | `close_position`            | `caller.require_auth()` + `caller==user`   | `user.require_auth()`                       | ✓      |
| 6 | TradeExecutor    | Oracle           | `get_price`                 | None (read-only, keeper-callable)          | None                                        | ✓      |
| 7 | TradeExecutor    | UserPortfolio    | `close_position_keeper`     | None (keeper-callable)                     | `caller.require_auth()` + `caller==executor`| ✓      |
| 8 | TradeExecutor    | Oracle           | `get_price`                 | None (read-only, keeper-callable)          | None                                        | ✓      |
| 9 | TradeExecutor    | UserPortfolio    | `close_position_keeper`     | None (keeper-callable)                     | `caller.require_auth()` + `caller==executor`| ✓      |
|10 | TradeExecutor    | SDEX Router      | `swap` (swap path)          | None (contract swaps own tokens)           | SEP-41 `approve` pre-authorises pull        | ✓      |
|11 | UserPortfolio    | SEP-41 Token     | `transfer`                  | `user.require_auth()` before call          | SEP-41 checks user auth                     | ✓      |
|12 | FeeCollector     | SEP-41 Token     | `transfer` (collect fee)    | `trader.require_auth()` before call        | SEP-41 checks trader auth                   | ✓      |
|13 | FeeCollector     | SEP-41 Token     | `transfer` (claim fees)     | `provider.require_auth()` before call      | SEP-41 checks contract auth                 | ✓      |
|14 | FeeCollector     | SEP-41 Token     | `transfer` (treasury)       | `admin.require_auth()` + timelock          | SEP-41 checks contract auth                 | ✓      |
|15 | SignalRegistry   | UserPortfolio    | `check_subscription`        | None (read-only)                           | None                                        | ✓      |

---

## Keeper Pattern (calls 7 & 9)

Stop-loss and take-profit triggers are **keeper-callable**: any address may invoke
`check_and_trigger_stop_loss` / `check_and_trigger_take_profit` without a user
signature. This is intentional — keepers are automated bots that monitor prices.

The position close is performed via `UserPortfolio::close_position_keeper`, a
dedicated entrypoint that:

1. Accepts a `caller: Address` parameter.
2. Calls `caller.require_auth()` — the TradeExecutor contract must authorise the
   sub-invocation (Soroban propagates contract auth automatically when the contract
   is the transaction source or is listed in `auth_entries`).
3. Verifies `caller == registered_trade_executor` (set by admin via
   `UserPortfolio::set_trade_executor`).

This prevents any contract other than the registered TradeExecutor from closing
positions via the keeper path, while still not requiring the user's signature.

### Deployment checklist

After deploying both contracts, the admin must call:

```
UserPortfolio::set_trade_executor(trade_executor_address)
```

Without this step, `close_position_keeper` will panic with "trade executor not set".

---

## No-Auth-Bypass Guarantee

No cross-contract call in this codebase allows an unauthenticated party to:

- Move user funds (all token transfers require the owner's `require_auth()`).
- Open positions on behalf of a user (requires `user.require_auth()`).
- Close user positions without either user auth or TradeExecutor-as-keeper auth.
- Access PREMIUM signals without a valid on-chain subscription.

The only calls that require no auth are read-only queries (`balance`, `get_price`,
`check_subscription`, `has_position`) and the keeper trigger path, which is
restricted to the registered TradeExecutor via `close_position_keeper`.
