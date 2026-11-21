#![cfg(test)]
//! Unit tests for the tax event log (Issue #658).
//!
//! Covers:
//! - Tax event fields are correct after close_position
//! - Tax event fields are correct after close_position_keeper
//! - Multiple events, all recorded in order
//! - get_tax_events timestamp range filtering
//! - get_tax_events pagination (offset + limit)
//! - Empty result for user with no events

use crate::{storage::DataKey, TaxEvent, UserPortfolio, UserPortfolioClient};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger as _},
    Address, Env, Vec,
};

// ── Mock oracle (always panics — positions don't need oracle for these tests) ──

#[contract]
struct OracleDummy;

#[contractimpl]
impl OracleDummy {
    pub fn get_price(_env: Env, _asset_pair: u32) -> stellar_swipe_common::OraclePrice {
        panic!("oracle not used in tax tests")
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let oracle = env.register(OracleDummy, ());
    let contract_id = env.register(UserPortfolio, ());
    let client = UserPortfolioClient::new(env, &contract_id);
    client.initialize(&admin, &oracle);
    // Register admin as trade executor so we can test keeper closes.
    client.set_trade_executor(&admin);
    (admin, contract_id)
}

fn provider(env: &Env) -> Address {
    Address::generate(env)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// A normal close records a tax event with the correct fields.
#[test]
fn close_position_records_tax_event() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    let id = client.open_position(&user, &500, &1_000);
    env.ledger().with_mut(|l| l.timestamp = 2_000);
    client.close_position(&user, &id, &150, &550i128, &7u32, &provider(&env), &0u64);

    let events = client.get_tax_events(&user, &0u64, &u64::MAX, &0u32, &100u32);
    assert_eq!(events.len(), 1);
    let e = events.get(0).unwrap();
    assert_eq!(e.position_id, id);
    assert_eq!(e.open_ts, 1_000);
    assert_eq!(e.close_ts, 2_000);
    assert_eq!(e.entry_price, 500);
    assert_eq!(e.exit_price, 550);
    assert_eq!(e.amount, 1_000);
    assert_eq!(e.realized_pnl, 150);
    assert_eq!(e.asset_pair, 7u32);
}

/// A keeper close records a tax event with exit_price=0 and realized_pnl=0.
#[test]
fn keeper_close_records_tax_event_with_zero_exit_price() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 500);
    let (admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    let id = client.open_position(&user, &300, &2_000);
    env.ledger().with_mut(|l| l.timestamp = 800);
    client.close_position_keeper(&admin, &user, &id, &3u32);

    let events = client.get_tax_events(&user, &0u64, &u64::MAX, &0u32, &100u32);
    assert_eq!(events.len(), 1);
    let e = events.get(0).unwrap();
    assert_eq!(e.position_id, id);
    assert_eq!(e.open_ts, 500);
    assert_eq!(e.close_ts, 800);
    assert_eq!(e.entry_price, 300);
    assert_eq!(e.exit_price, 0);
    assert_eq!(e.amount, 2_000);
    assert_eq!(e.realized_pnl, 0);
    assert_eq!(e.asset_pair, 3u32);
}

/// Multiple closes append events in chronological order.
#[test]
fn multiple_closes_append_events_in_order() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let p = provider(&env);

    for i in 0u64..3 {
        env.ledger().with_mut(|l| l.timestamp = (i + 1) * 1_000);
        let id = client.open_position(&user, &100, &1_000);
        env.ledger()
            .with_mut(|l| l.timestamp = (i + 1) * 1_000 + 500);
        client.close_position(&user, &id, &(i as i128 * 50), &120i128, &1u32, &p, &0u64);
    }

    let events = client.get_tax_events(&user, &0u64, &u64::MAX, &0u32, &100u32);
    assert_eq!(events.len(), 3);
    assert_eq!(events.get(0).unwrap().close_ts, 1_500);
    assert_eq!(events.get(1).unwrap().close_ts, 2_500);
    assert_eq!(events.get(2).unwrap().close_ts, 3_500);
}

/// Timestamp range filter returns only events whose close_ts falls in [from, to].
#[test]
fn get_tax_events_filters_by_timestamp_range() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let p = provider(&env);

    for ts in [100u64, 200, 300, 400, 500] {
        env.ledger().with_mut(|l| l.timestamp = ts - 10);
        let id = client.open_position(&user, &100, &1_000);
        env.ledger().with_mut(|l| l.timestamp = ts);
        client.close_position(&user, &id, &0, &100i128, &1u32, &p, &0u64);
    }

    let events = client.get_tax_events(&user, &200u64, &400u64, &0u32, &100u32);
    assert_eq!(events.len(), 3);
    assert_eq!(events.get(0).unwrap().close_ts, 200);
    assert_eq!(events.get(1).unwrap().close_ts, 300);
    assert_eq!(events.get(2).unwrap().close_ts, 400);
}

/// Pagination: offset skips leading events, limit caps the result count.
#[test]
fn get_tax_events_pagination_offset_and_limit() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let p = provider(&env);

    for i in 0u64..6 {
        env.ledger().with_mut(|l| l.timestamp = i * 100 + 10);
        let id = client.open_position(&user, &100, &1_000);
        env.ledger().with_mut(|l| l.timestamp = i * 100 + 50);
        client.close_position(&user, &id, &0, &100i128, &1u32, &p, &0u64);
    }

    // All 6 events: close_ts = 50, 150, 250, 350, 450, 550
    let page = client.get_tax_events(&user, &0u64, &u64::MAX, &2u32, &3u32);
    assert_eq!(page.len(), 3);
    assert_eq!(page.get(0).unwrap().close_ts, 250);
    assert_eq!(page.get(1).unwrap().close_ts, 350);
    assert_eq!(page.get(2).unwrap().close_ts, 450);
}

/// limit=0 returns an empty result.
#[test]
fn get_tax_events_limit_zero_returns_empty() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let p = provider(&env);

    env.ledger().with_mut(|l| l.timestamp = 100);
    let id = client.open_position(&user, &100, &1_000);
    env.ledger().with_mut(|l| l.timestamp = 200);
    client.close_position(&user, &id, &0, &100i128, &1u32, &p, &0u64);

    let result = client.get_tax_events(&user, &0u64, &u64::MAX, &0u32, &0u32);
    assert_eq!(result.len(), 0);
}

/// User with no closed positions has an empty tax event list.
#[test]
fn get_tax_events_empty_for_new_user() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    let events = client.get_tax_events(&user, &0u64, &u64::MAX, &0u32, &100u32);
    assert_eq!(events.len(), 0);
}

/// Two users' tax events are isolated from each other.
#[test]
fn tax_events_are_isolated_per_user() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);
    let p = provider(&env);

    env.ledger().with_mut(|l| l.timestamp = 100);
    let id_a = client.open_position(&user_a, &100, &1_000);
    let id_b = client.open_position(&user_b, &200, &2_000);

    env.ledger().with_mut(|l| l.timestamp = 200);
    client.close_position(&user_a, &id_a, &10, &110i128, &1u32, &p, &0u64);

    env.ledger().with_mut(|l| l.timestamp = 300);
    client.close_position(&user_b, &id_b, &-20, &190i128, &2u32, &p, &0u64);

    let events_a = client.get_tax_events(&user_a, &0u64, &u64::MAX, &0u32, &100u32);
    let events_b = client.get_tax_events(&user_b, &0u64, &u64::MAX, &0u32, &100u32);

    assert_eq!(events_a.len(), 1);
    assert_eq!(events_a.get(0).unwrap().realized_pnl, 10);
    assert_eq!(events_b.len(), 1);
    assert_eq!(events_b.get(0).unwrap().realized_pnl, -20);
}

/// Offset past the end of the list returns an empty result.
#[test]
fn offset_past_end_returns_empty() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let p = provider(&env);

    env.ledger().with_mut(|l| l.timestamp = 100);
    let id = client.open_position(&user, &100, &1_000);
    env.ledger().with_mut(|l| l.timestamp = 200);
    client.close_position(&user, &id, &0, &100i128, &1u32, &p, &0u64);

    let result = client.get_tax_events(&user, &0u64, &u64::MAX, &5u32, &100u32);
    assert_eq!(result.len(), 0);
}
