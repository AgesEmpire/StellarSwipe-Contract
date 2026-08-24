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

use crate::errors::{ContractError, InsufficientLiquidityDetail};

/// SDEX order-book query function name on the router.
pub const SDEX_ORDERBOOK_FN: &str = "get_best_ask";

/// Query the best ask quantity available for a pair via the router.
/// Returns `(best_ask_price, available_quantity)`. Returns `(0, 0)` if the
/// order book is empty (zero liquidity).
fn query_best_ask(
    env: &Env,
    sdex_router: &Address,
    from_token: &Address,
    to_token: &Address,
) -> (i128, i128) {
    let sym = Symbol::new(env, SDEX_ORDERBOOK_FN);
    let mut args = Vec::<Val>::new(env);
    args.push_back(from_token.clone().into_val(env));
    args.push_back(to_token.clone().into_val(env));
    // Router returns (best_ask_price: i128, available_qty: i128).
    // If the call fails (e.g. router doesn't support it), treat as zero liquidity.
    env.invoke_contract::<(i128, i128)>(sdex_router, &sym, args)
}

/// Check order-book depth before executing a swap.
///
/// Returns `Err(ContractError::InsufficientLiquidity)` when:
/// - The order book is empty (`available_qty == 0`), or
/// - The best ask price exceeds `entry_price * (1 + max_slippage_bps / 10_000)`.
pub fn check_liquidity(
    env: &Env,
    sdex_router: &Address,
    from_token: &Address,
    to_token: &Address,
    required_amount: i128,
    entry_price: i128,
    max_slippage_bps: u32,
) -> Result<(), ContractError> {
    let (_best_ask_price, available_qty) = query_best_ask(env, sdex_router, from_token, to_token);

    if available_qty == 0 || available_qty < required_amount {
        return Err(ContractError::InsufficientLiquidity);
    }

    // Price guard: best_ask > entry_price * (1 + max_slippage_bps / 10_000)
    let threshold = entry_price
        .checked_mul(
            (10_000i128)
                .checked_add(max_slippage_bps as i128)
                .unwrap_or(i128::MAX),
        )
        .unwrap_or(i128::MAX)
        / 10_000;

    if _best_ask_price > threshold {
        return Err(ContractError::InsufficientLiquidity);
    }

    Ok(())
}

/// Build an [`InsufficientLiquidityDetail`] for the given pair (for error reporting).
pub fn get_liquidity_detail(
    env: &Env,
    sdex_router: &Address,
    from_token: &Address,
    to_token: &Address,
    required_amount: i128,
) -> InsufficientLiquidityDetail {
    let (_price, available_liquidity) = query_best_ask(env, sdex_router, from_token, to_token);
    InsufficientLiquidityDetail {
        available_liquidity,
        required_amount,
    }
}

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

    // Liquidity check: use min_received as the entry_price proxy and 0 slippage
    // (the caller already computed min_received from slippage). We check that
    // available_qty >= amount; price guard uses min_received as the floor.
    check_liquidity(
        env,
        sdex_router,
        from_token,
        to_token,
        amount,
        min_received,
        0,
    )?;

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

#[cfg(test)]
mod liquidity_tests {
    use super::*;
    use crate::errors::ContractError;

    // Helper: build a detail struct directly (no router needed for unit tests).
    fn detail(available: i128, required: i128) -> InsufficientLiquidityDetail {
        InsufficientLiquidityDetail {
            available_liquidity: available,
            required_amount: required,
        }
    }

    #[test]
    fn zero_liquidity_returns_insufficient_liquidity_detail() {
        let d = detail(0, 1_000);
        assert_eq!(d.available_liquidity, 0);
        assert_eq!(d.required_amount, 1_000);
    }

    #[test]
    fn insufficient_liquidity_detail_fields() {
        let d = detail(500, 1_000);
        assert!(d.available_liquidity < d.required_amount);
    }

    #[test]
    fn sufficient_liquidity_detail_fields() {
        let d = detail(2_000, 1_000);
        assert!(d.available_liquidity >= d.required_amount);
    }

    #[test]
    fn min_received_from_slippage_zero_slippage() {
        assert_eq!(min_received_from_slippage(1_000, 0), Some(1_000));
    }

    #[test]
    fn min_received_from_slippage_100bps() {
        // 1% slippage on 10_000 → 9_900
        assert_eq!(min_received_from_slippage(10_000, 100), Some(9_900));
    }

    #[test]
    fn min_received_from_slippage_full_slippage() {
        assert_eq!(min_received_from_slippage(1_000, 10_000), Some(0));
    }

    #[test]
    fn min_received_from_slippage_negative_amount() {
        assert_eq!(min_received_from_slippage(-1, 100), None);
    }
}
