feature/copy-trade-balance-check
//! Pre-trade safety checks (position caps, balance, etc.).

//! Pre-trade safety checks (position caps, etc.).
main
//!
//! Copy trading consults the configured **user portfolio** contract for open position
//! counts via `get_open_position_count(user)`. Point this at your deployment’s portfolio
//! contract (any Soroban contract that exposes that function and symbol).

 feature/copy-trade-balance-check
use soroban_sdk::{token, Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::{ContractError, InsufficientBalanceDetail};

use soroban_sdk::{Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::ContractError;
 main

/// Default maximum open copy-trade positions per user (safety rail for novices).
pub const MAX_POSITIONS_PER_USER: u32 = 20;

 feature/copy-trade-balance-check
/// Default estimated fee budget (in token smallest units) included in the balance check.
/// Covers typical Soroban + downstream invocation costs; override via admin storage if needed.
pub const DEFAULT_ESTIMATED_COPY_TRADE_FEE: i128 = 500_000;

/// Portfolio entrypoint: `get_open_position_count(user: Address) -> u32`.
pub const GET_OPEN_POSITION_COUNT_FN: &str = "get_open_position_count";

/// Ensure `user` holds at least `amount + estimated_fee` of `token` (SEP-41 SAC balance).
///
/// `amount` must be positive; `estimated_fee` must be non‑negative. Compares **raw** on-chain
/// balance to **required** spend including fees so users see a clear shortfall before any
/// router / SDEX invocation.
pub fn check_user_balance(
    env: &Env,
    user: &Address,
    token: &Address,
    amount: i128,
    estimated_fee: i128,
) -> Result<(), InsufficientBalanceDetail> {
    let available = token::Client::new(env, token).balance(user);
    let Some(required) = amount.checked_add(estimated_fee) else {
        return Err(InsufficientBalanceDetail {
            required: i128::MAX,
            available,
        });
    };

    if available >= required {
        Ok(())
    } else {
        Err(InsufficientBalanceDetail {
            required,
            available,
        })
    }
}


/// Portfolio entrypoint: `get_open_position_count(user: Address) -> u32`.
pub const GET_OPEN_POSITION_COUNT_FN: &str = "get_open_position_count";

 main
/// Enforce per-user open position cap unless `user` is on the admin whitelist.
///
/// Call from [`crate::TradeExecutorContract::execute_copy_trade`] **before** any state
/// changes or downstream portfolio updates.
pub fn check_position_limit(
    env: &Env,
    user_portfolio: &Address,
    user: &Address,
    position_limit_exempt: bool,
) -> Result<(), ContractError> {
    if position_limit_exempt {
        return Ok(());
    }

    let sym = Symbol::new(env, GET_OPEN_POSITION_COUNT_FN);
    let mut args = Vec::<Val>::new(env);
    args.push_back(user.clone().into_val(env));

    let open_count: u32 = env.invoke_contract(user_portfolio, &sym, args);
    if open_count >= MAX_POSITIONS_PER_USER {
        return Err(ContractError::PositionLimitReached);
    }

    Ok(())
}
