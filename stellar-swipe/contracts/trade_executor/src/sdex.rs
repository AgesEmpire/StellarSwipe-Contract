//! SDEX / aggregator integration helpers.
//!
//! Stellar’s **classic SDEX** (order books, path payments) is not invoked as a single
//! host syscall from Soroban. Production integrations route swaps through a **Soroban
//! router contract** (aggregator, pool router, or protocol-specific entrypoint) that
//! performs the equivalent of a strict-send path payment and delivers output tokens.
//!
//! This module:
//! 1. Approves the router on the input Stellar Asset Contract (SAC) using
//!    [`soroban_sdk::token::Client`] (SEP-41).
//! 2. Calls the router with [`Env::invoke_contract`].
//! 3. Verifies **actual** credit on the output SAC via balance delta (not only the
//!    return value), and reverts with [`crate::errors::ContractError::SlippageExceeded`]
//!    when `actual_received < min_received`.

use soroban_sdk::{token, Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::ContractError;

/// Router entrypoint name invoked on `sdex_router`.
pub const SDEX_SWAP_FN: &str = "swap";

/// Minimum SAC allowance lifetime (ledgers) granted to the router.
const ROUTER_ALLOWANCE_LEDGERS: u32 = 1_000_000;

/// Compute minimum acceptable output for a strict-send style swap.
///
/// `min_received = amount * (10_000 - max_slippage_bps) / 10_000`
///
/// Returns `None` on overflow. If `max_slippage_bps >= 10_000`, returns `Some(0)`.
pub fn min_received_from_slippage(amount: i128, max_slippage_bps: u32) -> Option<i128> {
    if amount <= 0 {
        return None;
    }
    if max_slippage_bps >= 10_000 {
        return Some(0);
    }
    let num = (10_000u32).checked_sub(max_slippage_bps)? as i128;
    amount.checked_mul(num)?.checked_div(10_000)
}

/// Execute a swap by approving the router on `from_token` and invoking its `swap` function.
///
/// Expected router ABI (topics / ordering must match `invoke_contract` args):
///
/// ```text
/// swap(
///   pull_from: Address,   // SAC balance this swap debits (usually the caller contract)
///   from_token: Address,  // input SAC contract
///   to_token: Address,     // output SAC contract
///   amount_in: i128,
///   min_out: i128,         // router-level minimum; executor still enforces balance check
///   recipient: Address,    // receives output tokens (usually pull_from)
/// ) -> i128                // reported amount out (informational)
/// ```
///
/// The router should `transfer_from` `amount_in` from `pull_from` and `transfer`
/// output tokens to `recipient`.
pub fn execute_sdex_swap(
    env: &Env,
    sdex_router: &Address,
    from_token: &Address,
    to_token: &Address,
    amount: i128,
    min_received: i128,
) -> Result<i128, ContractError> {
    if amount <= 0 || min_received < 0 {
        return Err(ContractError::InvalidAmount);
    }

    let this = env.current_contract_address();
    let from_client = token::Client::new(env, from_token);
    let to_client = token::Client::new(env, to_token);

    let expiration = env
        .ledger()
        .sequence()
        .checked_add(ROUTER_ALLOWANCE_LEDGERS)
        .ok_or(ContractError::InvalidAmount)?;

    // SEP-41: current contract authorizes router to pull `amount` of from_token.
    from_client.approve(&this, sdex_router, &amount, &expiration);

    let balance_before = to_client.balance(&this);

    let swap_sym = Symbol::new(env, SDEX_SWAP_FN);
    let mut args = Vec::<Val>::new(env);
    args.push_back(this.clone().into_val(env));
    args.push_back(from_token.clone().into_val(env));
    args.push_back(to_token.clone().into_val(env));
    args.push_back(amount.into_val(env));
    args.push_back(min_received.into_val(env));
    args.push_back(this.clone().into_val(env));

    let _reported_out: i128 = env.invoke_contract(sdex_router, &swap_sym, args);

    let balance_after = to_client.balance(&this);
    let actual_received = balance_after.checked_sub(balance_before).unwrap_or(0);

    if actual_received < min_received {
        return Err(ContractError::SlippageExceeded);
    }

    Ok(actual_received)
}
