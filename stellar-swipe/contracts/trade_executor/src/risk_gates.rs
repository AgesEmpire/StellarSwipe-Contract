//! Pre-trade safety checks (position caps, balance, etc.).
//!
//! Copy trading consults the configured **user portfolio** contract via the batched
//! `validate_and_record(user, max_positions)` entrypoint, which atomically checks the
//! open-position count and records the new position in a **single** cross-contract call.

use soroban_sdk::{token, Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::{ContractError, InsufficientBalanceDetail};

/// Default maximum open copy-trade positions per user (safety rail for novices).
pub const MAX_POSITIONS_PER_USER: u32 = 20;

/// Maximum portfolio percentage allowed per copy trade (20% = 2000 bps).
pub const MAX_POSITION_PCT_BPS: u32 = 2_000;

/// Default estimated fee budget (in token smallest units) included in the balance check.
pub const DEFAULT_ESTIMATED_COPY_TRADE_FEE: i128 = 500_000;

/// Batched portfolio entrypoint: atomically validates the position cap and records the
/// copy position in one cross-contract call, replacing the old two-call pattern
/// (`get_open_position_count` + `record_copy_position`).
///
/// Expected ABI: `validate_and_record(user: Address, max_positions: u32) -> u32`
/// The portfolio panics (reverts) when `open_count >= max_positions`; we surface that
/// as [`ContractError::PositionLimitReached`].
pub const VALIDATE_AND_RECORD_FN: &str = "validate_and_record";

/// Ensure `user` holds at least `amount + estimated_fee` of `token` (SEP-41 SAC balance).
///
/// `amount` must be positive; `estimated_fee` must be non-negative.
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

/// Resolve the effective trade amount from an optional portfolio percentage.
///
/// - `Some(pct_bps)`: query the user's token balance as a proxy for portfolio value,
///   compute `balance * pct_bps / 10_000`, cap at `MAX_POSITION_PCT_BPS`.
///   Falls back to `explicit_amount` if the oracle is unavailable or the computed
///   amount is zero.
/// - `None`: return `explicit_amount` unchanged.
pub fn resolve_trade_amount(
    env: &Env,
    user: &Address,
    token: &Address,
    explicit_amount: i128,
    portfolio_pct_bps: Option<u32>,
    oracle: Option<Address>,
) -> Result<i128, ContractError> {
    let Some(mut pct_bps) = portfolio_pct_bps else {
        return Ok(explicit_amount);
    };

    // Cap at 20%.
    if pct_bps > MAX_POSITION_PCT_BPS {
        return Err(ContractError::PositionPctTooHigh);
    }

    // Oracle unavailable → fall back to explicit amount.
    if oracle.is_none() {
        return Ok(explicit_amount);
    }

    let portfolio_value = token::Client::new(env, token).balance(user);
    let computed = portfolio_value
        .checked_mul(pct_bps as i128)
        .and_then(|v| v.checked_div(10_000))
        .unwrap_or(0);

    if computed <= 0 {
        return Ok(explicit_amount);
    }

    Ok(computed)
}

/// Atomically enforce the per-user position cap **and** record the new copy position in a
/// **single** cross-contract call to the portfolio contract.
///
/// When `position_limit_exempt` is `true` the cap is passed as `u32::MAX` so the portfolio
/// always succeeds without a separate exemption check.
///
/// ## Optimization (Issue #306)
/// Replaces the previous two-call pattern:
///   - call A: `get_open_position_count(user) -> u32`
///   - call B: `record_copy_position(user)`
/// with a single batched call:
///   - call A: `validate_and_record(user, max_positions) -> u32`
///
/// Cross-contract call count in `execute_copy_trade`: **3 → 2** (−1 call, ≥33% reduction).
pub fn validate_and_record_position(
    env: &Env,
    user_portfolio: &Address,
    user: &Address,
    position_limit_exempt: bool,
) -> Result<(), ContractError> {
    let max_positions: u32 = if position_limit_exempt {
        u32::MAX
    } else {
        MAX_POSITIONS_PER_USER
    };

    let sym = Symbol::new(env, VALIDATE_AND_RECORD_FN);
    let mut args = Vec::<Val>::new(env);
    args.push_back(user.clone().into_val(env));
    args.push_back(max_positions.into_val(env));

    // try_invoke_contract returns Err when the callee panics (cap exceeded).
    let result = env.try_invoke_contract::<u32, soroban_sdk::Error>(user_portfolio, &sym, args);
    result
        .map_err(|_| ContractError::PositionLimitReached)?
        .map(|_| ())
        .map_err(|_| ContractError::PositionLimitReached)
}
