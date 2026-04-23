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

/// Maximum number of intermediate hops allowed by the Stellar protocol.
pub const MAX_PATH_HOPS: usize = 5;

/// Execute a swap via the SDEX router.
///
/// - `path` empty  → direct swap (`swap` entrypoint)
/// - `path` non-empty → multi-hop path payment (`swap_path` entrypoint); max 5 hops.
///
/// Router ABI for direct swap:
/// ```text
/// swap(pull_from, from_token, to_token, amount_in, min_out, recipient) -> i128
/// ```
/// Router ABI for path swap:
/// ```text
/// swap_path(pull_from, from_token, to_token, path: Vec<Address>, amount_in, min_out, recipient) -> i128
/// ```
pub fn execute_sdex_swap(
    env: &Env,
    sdex_router: &Address,
    from_token: &Address,
    to_token: &Address,
    amount: i128,
    min_received: i128,
    path: Vec<Address>,
) -> Result<i128, ContractError> {
    if amount <= 0 || min_received < 0 {
        return Err(ContractError::InvalidAmount);
    }
    if path.len() as usize > MAX_PATH_HOPS {
        return Err(ContractError::PathTooLong);
    }

    let this = env.current_contract_address();
    let from_client = token::Client::new(env, from_token);
    let to_client = token::Client::new(env, to_token);

    let expiration = env
        .ledger()
        .sequence()
        .checked_add(ROUTER_ALLOWANCE_LEDGERS)
        .ok_or(ContractError::InvalidAmount)?;

    from_client.approve(&this, sdex_router, &amount, &expiration);

    let balance_before = to_client.balance(&this);

    let mut args = Vec::<Val>::new(env);
    args.push_back(this.clone().into_val(env));
    args.push_back(from_token.clone().into_val(env));
    args.push_back(to_token.clone().into_val(env));

    let fn_sym = if path.is_empty() {
        args.push_back(amount.into_val(env));
        args.push_back(min_received.into_val(env));
        args.push_back(this.clone().into_val(env));
        Symbol::new(env, SDEX_SWAP_FN)
    } else {
        args.push_back(path.into_val(env));
        args.push_back(amount.into_val(env));
        args.push_back(min_received.into_val(env));
        args.push_back(this.clone().into_val(env));
        Symbol::new(env, "swap_path")
    };

    let _reported_out: i128 = env.invoke_contract(sdex_router, &fn_sym, args);

    let balance_after = to_client.balance(&this);
    let actual_received = balance_after.checked_sub(balance_before).unwrap_or(0);

    if actual_received < min_received {
        return Err(ContractError::SlippageExceeded);
    }

    Ok(actual_received)
}
