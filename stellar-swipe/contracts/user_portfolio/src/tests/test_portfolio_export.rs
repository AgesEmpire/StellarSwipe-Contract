#![cfg(test)]
//! Unit tests for portfolio export (Issue #912).
//!
//! Covers:
//! - Empty portfolio export returns zero counts and empty positions
//! - Export with open positions reflects correct counts and P&L
//! - Export with closed positions shows correct closed count
//! - Snapshot count is included in export
//! - Export is read-only (no state mutation)

use crate::{PortfolioExport, UserPortfolio, UserPortfolioClient};
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
        panic!("oracle not used in export tests")
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
    (admin, contract_id)
}

fn provider(env: &Env) -> Address {
    Address::generate(env)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Empty portfolio: all counts are zero, no open positions.
#[test]
fn export_empty_portfolio() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    let export = client.export_portfolio(&user);

    assert_eq!(export.realized_pnl, 0);
    assert_eq!(export.unrealized_pnl, Some(0));
    assert_eq!(export.total_pnl, 0);
    assert_eq!(export.roi_bps, 0);
    assert_eq!(export.open_position_count, 0);
    assert_eq!(export.closed_position_count, 0);
    assert_eq!(export.snapshot_count, 0);
    assert_eq!(export.open_positions.len(), 0);
}

/// Portfolio with open positions: counts and P&L reflect the open state.
#[test]
fn export_with_open_positions() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.open_position(&user, &100, &1_000);
    client.open_position(&user, &200, &500);

    let export = client.export_portfolio(&user);

    assert_eq!(export.realized_pnl, 0);
    // Oracle dummy panics, so unrealized is None for open positions.
    assert_eq!(export.total_pnl, 0);
    assert_eq!(export.roi_bps, 0);
    assert_eq!(export.open_position_count, 2);
    assert_eq!(export.closed_position_count, 0);
    assert_eq!(export.snapshot_count, 0);
    assert_eq!(export.open_positions.len(), 2);

    // First position
    let p0 = export.open_positions.get(0).unwrap();
    assert_eq!(p0.position_id, 1);
    assert_eq!(p0.position.entry_price, 100);
    assert_eq!(p0.position.amount, 1_000);

    // Second position
    let p1 = export.open_positions.get(1).unwrap();
    assert_eq!(p1.position_id, 2);
    assert_eq!(p1.position.entry_price, 200);
    assert_eq!(p1.position.amount, 500);
}

/// Portfolio with closed positions: closed count is reflected in export.
#[test]
fn export_with_closed_positions() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let p = provider(&env);

    client.open_position(&user, &100, &1_000);
    client.open_position(&user, &100, &500);

    env.ledger().with_mut(|l| l.timestamp = 2_000);
    client.close_position(&user, &1, &150, &110i128, &1u32, &p, &0u64);

    let export = client.export_portfolio(&user);

    assert_eq!(export.realized_pnl, 150);
    assert_eq!(export.open_position_count, 1);
    assert_eq!(export.closed_position_count, 1);
    assert_eq!(export.open_positions.len(), 1);

    // Only the open position remains in open_positions
    let pos = export.open_positions.get(0).unwrap();
    assert_eq!(pos.position_id, 2);
}

/// Snapshot count is reflected in the export.
#[test]
fn export_includes_snapshot_count() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 100);
    client.take_snapshot(&user);
    env.ledger().with_mut(|l| l.timestamp = 200);
    client.take_snapshot(&user);
    env.ledger().with_mut(|l| l.timestamp = 300);
    client.take_snapshot(&user);

    let export = client.export_portfolio(&user);

    assert_eq!(export.snapshot_count, 3);
}

/// Export is read-only: calling it does not mutate storage.
#[test]
fn export_is_read_only() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    // Export before any state changes.
    let before = client.export_portfolio(&user);
    assert_eq!(before.snapshot_count, 0);

    // Export again — state should be identical (no writes from export).
    let after = client.export_portfolio(&user);
    assert_eq!(after.snapshot_count, 0);
    assert_eq!(after.open_position_count, 0);
}

/// Two users' exports are isolated from each other.
#[test]
fn export_is_isolated_per_user() {
    let env = Env::default();
    let (_admin, contract_id) = setup(&env);
    let client = UserPortfolioClient::new(&env, &contract_id);
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);

    client.open_position(&user_a, &100, &1_000);

    let export_a = client.export_portfolio(&user_a);
    let export_b = client.export_portfolio(&user_b);

    assert_eq!(export_a.open_position_count, 1);
    assert_eq!(export_b.open_position_count, 0);
}
