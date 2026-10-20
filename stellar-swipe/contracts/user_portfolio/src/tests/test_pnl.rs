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
    assert_eq!(pnl.realized_pnl, 15); // 20 + (-5)
    assert_eq!(pnl.total_pnl, 15);
    // roi_bps = 15 * 10_000 / 20 = 7500
    assert_eq!(pnl.roi_bps, 7_500);
}

// ── Notification event tests ──────────────────────────────────────────────────

/// close_position emits EvtPositionClosed with all required notification fields.
#[test]
fn close_position_emits_position_closed_event() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::TryFromVal;

    let (env, user, client) = setup(120);
    env.ledger().with_mut(|l| l.timestamp = 5000);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &10);
    client.close_position(&user, &1, &20, &120i128, &0u32, &provider, &0u64);

    // Find the position_closed event.
    let events = env.events().all();
    let pos_closed = events.iter().find(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        if topics.len() < 2 {
            return false;
        }
        let t1 = soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap());
        t1.map(|s| s == soroban_sdk::Symbol::new(&env, "position_closed"))
            .unwrap_or(false)
    });

    assert!(pos_closed.is_some(), "EvtPositionClosed not emitted");
    let evt: shared::events::EvtPositionClosed =
        shared::events::EvtPositionClosed::try_from_val(&env, &pos_closed.unwrap().2).unwrap();

    assert_eq!(evt.trade_id, 1);
    assert_eq!(evt.exit_price, 120);
    assert_eq!(evt.realized_pnl, 20);
    assert_eq!(evt.timestamp, 5000);
    assert!(!evt.action_required);
    assert_eq!(evt.schema_version, shared::events::SCHEMA_VERSION);
}

// ── FIFO cost-basis tests ─────────────────────────────────────────────────────

/// FIFO ordering: the first lot added is consumed first.
/// Lot A @ entry 100 (amount 10) added before lot B @ entry 200 (amount 10).
/// Closing 10 at exit 150 must consume lot A, not lot B.
///   pnl_A = (150-100)*10/100 = 5
///   pnl_B = (150-200)*10/200 = -2  (would be this if ordering were wrong)
#[test]
fn fifo_consumes_first_lot_first() {
    let (env, user, client) = setup(150);

    client.add_cost_lot(&user, &100, &10); // lot A
    client.add_cost_lot(&user, &200, &10); // lot B

    let pnl = client.close_fifo(&user, &10, &150);
    assert_eq!(pnl, 5); // lot A consumed, not lot B (-2)

    let summary = client.get_pnl(&user);
    assert_eq!(summary.realized_pnl, 5);
}

/// Close spans two lots: close_amount exceeds the first lot's size so the
/// second lot is partially consumed after the first is exhausted.
///   Lot A @ 100, amount 1000
///   Lot B @ 200, amount 1000
///   Close 1500 @ exit 150
///   pnl_A = (150-100)*1000/100 = 500
///   pnl_B_partial = (150-200)*500/200 = -125
///   total = 375
#[test]
fn fifo_close_spans_two_lots() {
    let (env, user, client) = setup(150);

    client.add_cost_lot(&user, &100, &1000); // lot A
    client.add_cost_lot(&user, &200, &1000); // lot B
    client.add_cost_lot(&user, &150, &1000); // lot C (should not be touched)

    let pnl = client.close_fifo(&user, &1500, &150);
    assert_eq!(pnl, 375); // 500 + (-125)

    let summary = client.get_pnl(&user);
    assert_eq!(summary.realized_pnl, 375);
}

/// Partial lot consumption: first close takes half of lot A;
/// second close takes the remaining half. Both come from lot A (FIFO), not lot B.
///   Lot A @ 100, amount 20
///   Lot B @ 200, amount 20
///   Close 10 @ 120: pnl = (120-100)*10/100 = 2  (from lot A)
///   Close 10 @ 120: pnl = (120-100)*10/100 = 2  (still from lot A's remainder)
#[test]
fn fifo_partial_lot_across_two_closes() {
    let (env, user, client) = setup(120);

    client.add_cost_lot(&user, &100, &20); // lot A
    client.add_cost_lot(&user, &200, &20); // lot B

    let pnl1 = client.close_fifo(&user, &10, &120);
    assert_eq!(pnl1, 2); // from lot A

    let pnl2 = client.close_fifo(&user, &10, &120);
    assert_eq!(pnl2, 2); // still from lot A (remaining half)

    let summary = client.get_pnl(&user);
    assert_eq!(summary.realized_pnl, 4); // 2 + 2
}

/// Three lots closed sequentially, each close consuming exactly one lot.
/// Validates strict FIFO: A consumed before B before C.
///   Lot A @ 100, amount 100  → close @ 110: pnl = (110-100)*100/100 = 10
///   Lot B @ 120, amount 100  → close @ 110: pnl = (110-120)*100/120 = -8
///   Lot C @ 80,  amount 100  → close @ 110: pnl = (110-80)*100/80   = 37
///   Cumulative = 39
#[test]
fn fifo_three_lots_sequential_closes_validate_ordering() {
    let (env, user, client) = setup(110);

    client.add_cost_lot(&user, &100, &100); // lot A
    client.add_cost_lot(&user, &120, &100); // lot B
    client.add_cost_lot(&user, &80, &100); // lot C

    let pnl_a = client.close_fifo(&user, &100, &110);
    assert_eq!(pnl_a, 10);

    let pnl_b = client.close_fifo(&user, &100, &110);
    assert_eq!(pnl_b, -8); // loss from lot B

    let pnl_c = client.close_fifo(&user, &100, &110);
    assert_eq!(pnl_c, 37);

    let summary = client.get_pnl(&user);
    assert_eq!(summary.realized_pnl, 39); // 10 + (-8) + 37
}

/// close_position emits EvtPositionClosed even for a loss (pnl <= 0).
#[test]
fn close_position_emits_position_closed_event_on_loss() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::TryFromVal;

    let (env, user, client) = setup(80);
    let provider = dummy_provider(&env);

    client.open_position(&user, &100, &10);
    client.close_position(&user, &1, &-20, &80i128, &0u32, &provider, &0u64);

    let events = env.events().all();
    let pos_closed = events.iter().find(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        if topics.len() < 2 {
            return false;
        }
        let t1 = soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap());
        t1.map(|s| s == soroban_sdk::Symbol::new(&env, "position_closed"))
            .unwrap_or(false)
    });

    assert!(
        pos_closed.is_some(),
        "EvtPositionClosed not emitted on loss"
    );
    let evt: shared::events::EvtPositionClosed =
        shared::events::EvtPositionClosed::try_from_val(&env, &pos_closed.unwrap().2).unwrap();
    assert_eq!(evt.realized_pnl, -20);
    assert!(!evt.action_required);
}

// ── Instruction-budget regression snapshots (Issue #budget) ───────────────────

use stellar_swipe_common::budget_regression::measure_and_emit;

#[test]
fn record_trade_budget_regression() {
    let (env, user, client) = setup(100);
    let provider = dummy_provider(&env);

    env.budget().reset_tracker();
    client.open_position(&user, &100, &10);
    let instructions = env.budget().cpu_instruction_cost();
    measure_and_emit("user_portfolio.record_trade", 5_000_000, instructions);
}
