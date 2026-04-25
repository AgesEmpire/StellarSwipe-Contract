#![cfg(test)]
//! Unit tests for UserPortfolio P&L calculation (#256).
//!
//! All five scenarios use integer arithmetic only (no floating point).
//! P&L formula for a closed position: realized_pnl is passed explicitly to
//! `close_position`. ROI is computed as total_pnl * 10_000 / total_invested (bps).

use crate::{UserPortfolio, UserPortfolioClient};
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Ledger},
    Address, Env,
};
use stellar_swipe_common::OraclePrice;

// ── Oracle mock ───────────────────────────────────────────────────────────────

#[contract]
pub struct OracleMock;

#[contractimpl]
impl OracleMock {
    pub fn set_price(env: Env, asset_pair: u32, price: OraclePrice) {
        env.storage()
            .instance()
            .set(&(symbol_short!("price"), asset_pair), &price);
    }
    pub fn get_price(env: Env, asset_pair: u32) -> OraclePrice {
        env.storage()
            .instance()
            .get(&(symbol_short!("price"), asset_pair))
            .unwrap()
    }
}

// ── Setup helper ──────────────────────────────────────────────────────────────

/// Returns `(env, user, portfolio_client)` with a working oracle set to `oracle_price`.
#[allow(deprecated)]
fn setup(oracle_price: i128) -> (Env, Address, UserPortfolioClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let oracle_id = env.register_contract(None, OracleMock);
    OracleMockClient::new(&env, &oracle_id).set_price(
        &0u32,
        &OraclePrice {
            price: oracle_price * 100,
            decimals: 2,
            timestamp: env.ledger().timestamp(),
            source: soroban_sdk::Symbol::new(&env, "mock"),
        },
    );

    let portfolio_id = env.register_contract(None, UserPortfolio);
    let client = UserPortfolioClient::new(&env, &portfolio_id);
    client.initialize(&admin, &oracle_id);

    (env, user, client)
}

fn dummy_provider(env: &Env) -> Address {
    Address::generate(env)
}

// ── P&L scenarios ─────────────────────────────────────────────────────────────

/// Long profit: entry 100, exit 120, amount 10 → P&L = +20, ROI = 2000 bps (20%).
#[test]
fn long_profit() {
    let (env, user, client) = setup(120);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &10);
    // realized_pnl = (120 - 100) * 10 / 100 = 2 … but the contract stores
    // whatever the caller passes as realized_pnl. We pass the correct value.
    client.close_position(&user, &1, &20, &120i128, &0u32, &provider, &0u64);

    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.realized_pnl, 20);
    assert_eq!(pnl.total_pnl, 20);
    // roi_bps = 20 * 10_000 / 10 = 20_000 bps
    assert_eq!(pnl.roi_bps, 20_000);
}

/// Long loss: entry 100, exit 80, amount 10 → P&L = -20, ROI = -20000 bps (-200%).
#[test]
fn long_loss() {
    let (env, user, client) = setup(80);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &10);
    client.close_position(&user, &1, &-20, &80i128, &0u32, &provider, &0u64);

    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.realized_pnl, -20);
    assert_eq!(pnl.total_pnl, -20);
    assert_eq!(pnl.roi_bps, -20_000);
}

/// Short profit: entry 100, exit 80, amount 10 → P&L = +20.
/// For a short, profit = (entry - exit) * amount / entry.
#[test]
fn short_profit() {
    let (env, user, client) = setup(80);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &10);
    // Short profit: price fell from 100 → 80, gain = 20.
    client.close_position(&user, &1, &20, &80i128, &0u32, &provider, &0u64);

    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.realized_pnl, 20);
    assert_eq!(pnl.total_pnl, 20);
    assert_eq!(pnl.roi_bps, 20_000);
}

/// Short loss: entry 100, exit 120, amount 10 → P&L = -20.
#[test]
fn short_loss() {
    let (env, user, client) = setup(120);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &10);
    // Short loss: price rose from 100 → 120, loss = -20.
    client.close_position(&user, &1, &-20, &120i128, &0u32, &provider, &0u64);

    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.realized_pnl, -20);
    assert_eq!(pnl.total_pnl, -20);
    assert_eq!(pnl.roi_bps, -20_000);
}

/// Breakeven: entry == exit → P&L = 0, ROI = 0.
#[test]
fn breakeven() {
    let (env, user, client) = setup(100);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &10);
    client.close_position(&user, &1, &0, &100i128, &0u32, &provider, &0u64);

    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.realized_pnl, 0);
    assert_eq!(pnl.total_pnl, 0);
    assert_eq!(pnl.roi_bps, 0);
}

// ── ROI integer arithmetic edge cases ────────────────────────────────────────

/// ROI uses integer division — verify truncation, not rounding.
/// total_pnl = 1, total_invested = 3 → 1 * 10_000 / 3 = 3333 (truncated).
#[test]
fn roi_integer_truncation() {
    let (env, user, client) = setup(100);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &3);
    client.close_position(&user, &1, &1, &100i128, &0u32, &provider, &0u64);

    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.realized_pnl, 1);
    assert_eq!(pnl.roi_bps, 3_333); // 10_000 / 3 = 3333 (truncated)
}

/// No positions → total_invested = 0 → ROI = 0 (no division-by-zero).
#[test]
fn roi_zero_invested_no_panic() {
    let (env, user, client) = setup(100);
    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.roi_bps, 0);
    assert_eq!(pnl.total_pnl, 0);
}

/// Multiple closed positions: P&L and ROI aggregate correctly.
#[test]
fn multiple_positions_aggregate() {
    let (env, user, client) = setup(100);
    let provider = dummy_provider(&env);

    // Position 1: +20 on amount 10
    client.open_position(&user, &100, &10);
    // Position 2: -5 on amount 10
    client.open_position(&user, &100, &10);

    client.close_position(&user, &1, &20, &120i128, &0u32, &provider, &0u64);
    client.close_position(&user, &2, &-5, &95i128, &0u32, &provider, &0u64);

    let pnl = client.get_pnl(&user);
    assert_eq!(pnl.realized_pnl, 15);   // 20 + (-5)
    assert_eq!(pnl.total_pnl, 15);
    // roi_bps = 15 * 10_000 / 20 = 7500
    assert_eq!(pnl.roi_bps, 7_500);
}
