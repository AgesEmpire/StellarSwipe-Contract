//! Oracle staleness tests for TradeExecutor.
//!
//! Verifies that `check_and_trigger_stop_loss` (and by extension any path that
//! calls `fetch_current_price`) correctly enforces the staleness threshold:
//!
//! | Scenario                          | Expected result        |
//! |-----------------------------------|------------------------|
//! | Fresh price (1 s old)             | trade proceeds (Ok)    |
//! | Price exactly at max-age boundary | trade proceeds (Ok)    |
//! | Price 1 s past max age            | OraclePriceStale       |
//! | No price ever published           | OracleUnavailable      |

#![cfg(test)]

extern crate std;

use crate::errors::ContractError;
use crate::triggers::MAX_ORACLE_PRICE_AGE_SECS;
use crate::{TradeExecutorContract, TradeExecutorContractClient};
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

// ── Mock oracle with timestamp control ───────────────────────────────────────

#[contract]
pub struct StaleOracle;

#[contractimpl]
impl StaleOracle {
    /// Store a price together with an explicit publish timestamp.
    pub fn set_price_at(env: Env, price: i128, timestamp: u64) {
        env.storage()
            .instance()
            .set(&symbol_short!("price"), &price);
        env.storage()
            .instance()
            .set(&symbol_short!("pts"), &timestamp);
    }

    pub fn get_price(env: Env, _asset_pair: u32) -> i128 {
        env.storage()
            .instance()
            .get(&symbol_short!("price"))
            .unwrap_or(0)
    }

    /// Returns 0 when no price has ever been published (signals OracleUnavailable).
    pub fn get_price_timestamp(env: Env, _asset_pair: u32) -> u64 {
        env.storage()
            .instance()
            .get(&symbol_short!("pts"))
            .unwrap_or(0u64)
    }
}

// ── Mock portfolio (keeper-callable close) ────────────────────────────────────

#[contract]
pub struct StalePortfolio;

#[contractimpl]
impl StalePortfolio {
    pub fn close_position_keeper(
        env: Env,
        _caller: Address,
        _user: Address,
        trade_id: u64,
        _asset_pair: u32,
    ) {
        env.storage()
            .instance()
            .set(&symbol_short!("closed"), &trade_id);
    }
}

// ── Setup ─────────────────────────────────────────────────────────────────────

const BASE_TS: u64 = 1_000_000;

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(BASE_TS);

    let admin = Address::generate(&env);
    let oracle_id = env.register(StaleOracle, ());
    let portfolio_id = env.register(StalePortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.add_oracle(&oracle_id);
    exec.set_oracle(&oracle_id);
    exec.set_stop_loss_portfolio(&portfolio_id);

    (env, exec_id, oracle_id, portfolio_id)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Scenario 1: price published 1 second ago — trade must proceed.
#[test]
fn fresh_price_trade_proceeds() {
    let (env, exec_id, oracle_id, _portfolio_id) = setup();
    let user = Address::generate(&env);

    // Publish price 1 second before "now".
    StaleOracleClient::new(&env, &oracle_id).set_price_at(&50, &(BASE_TS - 1));

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_stop_loss_price(&user, &1u64, &100);

    // Price (50) <= stop_loss (100) → would trigger if fresh; must not return an error.
    let result = exec.try_check_and_trigger_stop_loss(&user, &1u64, &0u32);
    assert!(
        result.is_ok(),
        "fresh price must not return an error: {result:?}"
    );
    assert!(
        result.unwrap().unwrap(),
        "stop-loss must trigger on fresh price"
    );
}

/// Scenario 2: price published exactly `MAX_ORACLE_PRICE_AGE_SECS` seconds ago — still valid.
#[test]
fn price_at_max_age_boundary_trade_proceeds() {
    let (env, exec_id, oracle_id, _portfolio_id) = setup();
    let user = Address::generate(&env);

    // Exactly at the boundary: age == MAX_ORACLE_PRICE_AGE_SECS.
    let publish_ts = BASE_TS - MAX_ORACLE_PRICE_AGE_SECS;
    StaleOracleClient::new(&env, &oracle_id).set_price_at(&50, &publish_ts);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_stop_loss_price(&user, &2u64, &100);

    let result = exec.try_check_and_trigger_stop_loss(&user, &2u64, &0u32);
    assert!(
        result.is_ok(),
        "price at exact max-age boundary must not be stale: {result:?}"
    );
    assert!(result.unwrap().unwrap());
}

/// Scenario 3: price published 1 second past max age — must return OraclePriceStale.
#[test]
fn price_one_second_past_max_age_returns_stale() {
    let (env, exec_id, oracle_id, _portfolio_id) = setup();
    let user = Address::generate(&env);

    // One second beyond the threshold.
    let publish_ts = BASE_TS - MAX_ORACLE_PRICE_AGE_SECS - 1;
    StaleOracleClient::new(&env, &oracle_id).set_price_at(&50, &publish_ts);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_stop_loss_price(&user, &3u64, &100);

    let result = exec.try_check_and_trigger_stop_loss(&user, &3u64, &0u32);
    assert!(
        matches!(result, Err(Ok(ContractError::OraclePriceStale))),
        "expected OraclePriceStale, got: {result:?}"
    );
}

/// Scenario 4: no price ever published — must return OracleUnavailable.
#[test]
fn no_price_returns_oracle_unavailable() {
    let (env, exec_id, _oracle_id, _portfolio_id) = setup();
    let user = Address::generate(&env);

    // Oracle has no price set (timestamp == 0).
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_stop_loss_price(&user, &4u64, &100);

    let result = exec.try_check_and_trigger_stop_loss(&user, &4u64, &0u32);
    assert!(
        matches!(result, Err(Ok(ContractError::OracleUnavailable))),
        "expected OracleUnavailable, got: {result:?}"
    );
}
