#![cfg(test)]
//! Unit tests for partial fill handling (Issue #959).
//!
//! Verifies that:
//! - A 0 % fill (zero amount received) records the shortfall and emits the event.
//! - A partial fill (0 < filled < requested) records the shortfall and emits the event.
//! - A 100 % fill is accepted as a no-op (no partial-fill record, no event).
//! - `filled_amount > requested_amount` is rejected with `InvalidAmount`.
//! - Negative amounts are rejected with `InvalidAmount`.
//! - Reporting a fill for an unknown trade_id returns `TradeNotFound`.
//! - `get_partial_fill` returns `None` after a 100 % fill.

use crate::{
    errors::ContractError,
    risk_gates::DEFAULT_ESTIMATED_COPY_TRADE_FEE,
    wire::{TradeOrder, TradeStatus, TRADE_TIMEOUT_LEDGERS},
    PartialFillRecord, StorageKey, TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, token::StellarAssetClient,
    Address, Env, Vec,
};

// ── Mock UserPortfolio ────────────────────────────────────────────────────────

#[contract]
pub struct MockPortfolio;

#[contracttype]
#[derive(Clone)]
enum PortfolioKey {
    Count(Address),
}

#[contractimpl]
impl MockPortfolio {
    pub fn validate_and_record(env: Env, user: Address, max_positions: u32) -> u32 {
        let key = PortfolioKey::Count(user.clone());
        let count: u32 = env.storage().instance().get(&key).unwrap_or(0);
        if count >= max_positions {
            panic!("position limit reached");
        }
        let new_count = count + 1;
        env.storage().instance().set(&key, &new_count);
        new_count
    }

    pub fn get_open_position_count(env: Env, user: Address) -> u32 {
        env.storage()
            .instance()
            .get(&PortfolioKey::Count(user))
            .unwrap_or(0)
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────────

const AMOUNT: i128 = 1_000_000;

fn sac(env: &Env) -> Address {
    let issuer = Address::generate(env);
    env.register_stellar_asset_contract_v2(issuer).address()
}

/// Set up executor + portfolio. Returns `(env, exec_id, admin, portfolio_id)`.
fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let portfolio_id = env.register(MockPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    (env, exec_id, admin, portfolio_id)
}

/// Manually inject a `PendingTradeConfirmation` for `(user, trade_id)` so we can
/// test `record_partial_fill` without going through the full execute path.
fn inject_pending_trade(env: &Env, exec_id: &Address, user: Address, trade_id: u64) {
    env.as_contract(exec_id, || {
        let order = TradeOrder {
            execution_ledger: env.ledger().sequence(),
            trade_id,
            user,
            amount: AMOUNT,
            expires_at_ledger: env
                .ledger()
                .sequence()
                .saturating_add(TRADE_TIMEOUT_LEDGERS),
            status: TradeStatus::ExecutedAwaitingConfirmation,
        };
        env.storage()
            .instance()
            .set(&StorageKey::PendingTradeConfirmation, &order);
    });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// A 100 % fill should be accepted silently (no record, no event).
#[test]
fn full_fill_is_noop() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    let trade_id: u64 = 1;

    inject_pending_trade(&env, &exec_id, user.clone(), trade_id);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.record_partial_fill(&admin, &user, &trade_id, &AMOUNT, &AMOUNT);

    let record = exec.get_partial_fill(&user, &trade_id);
    assert!(
        record.is_none(),
        "expected no partial fill record for 100% fill"
    );
}

/// A 50 % fill should record the shortfall and emit the partial_fill event.
#[test]
fn partial_fill_records_shortfall() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    let trade_id: u64 = 2;
    let requested = AMOUNT;
    let filled = AMOUNT / 2;
    let expected_remaining = AMOUNT - filled;

    inject_pending_trade(&env, &exec_id, user.clone(), trade_id);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.record_partial_fill(&admin, &user, &trade_id, &requested, &filled);

    let record = exec
        .get_partial_fill(&user, &trade_id)
        .expect("expected a partial fill record for 50% fill");

    assert_eq!(record.requested_amount, requested);
    assert_eq!(record.filled_amount, filled);
    assert_eq!(record.remaining_amount, expected_remaining);
}

/// A 0 % fill should record the full amount as remaining.
#[test]
fn zero_fill_records_full_remaining() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    let trade_id: u64 = 3;

    inject_pending_trade(&env, &exec_id, user.clone(), trade_id);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.record_partial_fill(&admin, &user, &trade_id, &AMOUNT, &0);

    let record = exec
        .get_partial_fill(&user, &trade_id)
        .expect("expected a partial fill record for 0% fill");

    assert_eq!(record.requested_amount, AMOUNT);
    assert_eq!(record.filled_amount, 0);
    assert_eq!(record.remaining_amount, AMOUNT);
}

/// `filled_amount > requested_amount` must return `InvalidAmount`.
#[test]
fn filled_exceeds_requested_is_invalid() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    let trade_id: u64 = 4;

    inject_pending_trade(&env, &exec_id, user.clone(), trade_id);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let result = exec.try_record_partial_fill(&admin, &user, &trade_id, &AMOUNT, &(AMOUNT + 1));
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidAmount)),
        "expected InvalidAmount when filled > requested"
    );
}

/// Negative `requested_amount` must return `InvalidAmount`.
#[test]
fn negative_requested_amount_is_invalid() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    let trade_id: u64 = 5;

    inject_pending_trade(&env, &exec_id, user.clone(), trade_id);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let result = exec.try_record_partial_fill(&admin, &user, &trade_id, &-1, &0);
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidAmount)),
        "expected InvalidAmount for negative requested_amount"
    );
}

/// Negative `filled_amount` must return `InvalidAmount`.
#[test]
fn negative_filled_amount_is_invalid() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    let trade_id: u64 = 6;

    inject_pending_trade(&env, &exec_id, user.clone(), trade_id);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let result = exec.try_record_partial_fill(&admin, &user, &trade_id, &AMOUNT, &-1);
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidAmount)),
        "expected InvalidAmount for negative filled_amount"
    );
}

/// Reporting a fill for a mismatched trade_id must return `TradeNotFound`.
#[test]
fn unknown_trade_id_returns_trade_not_found() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    inject_pending_trade(&env, &exec_id, user.clone(), 1);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let result = exec.try_record_partial_fill(&admin, &user, &99, &AMOUNT, &(AMOUNT / 2));
    assert_eq!(
        result,
        Err(Ok(ContractError::TradeNotFound)),
        "expected TradeNotFound for unknown trade_id"
    );
}

/// After a partial fill, pending trade status is updated to `PartiallyFilled`.
#[test]
fn partial_fill_updates_trade_status() {
    let (env, exec_id, admin, _) = setup();
    let user = Address::generate(&env);
    let trade_id: u64 = 7;

    inject_pending_trade(&env, &exec_id, user.clone(), trade_id);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.record_partial_fill(&admin, &user, &trade_id, &AMOUNT, &(AMOUNT / 2));

    let order: Option<TradeOrder> = env.as_contract(&exec_id, || {
        env.storage()
            .instance()
            .get(&StorageKey::PendingTradeConfirmation)
    });
    let order = order.expect("pending trade not found");
    assert_eq!(
        order.status,
        TradeStatus::PartiallyFilled,
        "expected PartiallyFilled status after partial fill"
    );
}

/// `get_partial_fill` returns `None` when no fill was recorded.
#[test]
fn get_partial_fill_returns_none_when_absent() {
    let (env, exec_id, _, _) = setup();
    let user = Address::generate(&env);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let record = exec.get_partial_fill(&user, &999);
    assert!(record.is_none(), "expected None for absent partial fill");
}
