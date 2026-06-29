#![cfg(test)]
//! Unit tests for trade cancellation grace period (Issue #702).
//!
//! Covers:
//! - Setting and reading the grace period config
//! - Queuing a trade and cancelling within the grace period
//! - Cancellation after the grace period has elapsed (should fail)
//! - Cancellation by a non-owner (should fail)
//! - Executing queued trades after grace period elapses
//! - Grace period of 0 (trades eligible immediately)

use crate::{
    errors::ContractError,
    TradeExecutorContract, TradeExecutorContractClient,
    QueuedTrade, DEFAULT_GRACE_PERIOD_LEDGERS,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
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
        if count >= max_positions { panic!("position limit reached"); }
        let new_count = count + 1;
        env.storage().instance().set(&key, &new_count);
        new_count
    }

    pub fn get_open_position_count(env: Env, user: Address) -> u32 {
        env.storage().instance().get(&PortfolioKey::Count(user)).unwrap_or(0)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const AMOUNT: i128 = 1_000_000;

fn sac(env: &Env) -> Address {
    let issuer = Address::generate(env);
    env.register_stellar_asset_contract_v2(issuer).address()
}

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let portfolio_id = env.register(MockPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);
    (env, exec_id, portfolio_id)
}

fn funded_user(env: &Env, token: &Address) -> Address {
    let user = Address::generate(env);
    StellarAssetClient::new(env, token).mint(&user, &(AMOUNT * 10));
    user
}
// ── Tests ─────────────────────────────────────────────────────────────────────

/// Default grace period is DEFAULT_GRACE_PERIOD_LEDGERS.
#[test]
fn default_grace_period_is_10() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    assert_eq!(exec.get_trade_grace_period(), DEFAULT_GRACE_PERIOD_LEDGERS);
}

/// Admin can configure the grace period.
#[test]
fn admin_can_set_grace_period() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_trade_grace_period(&25);
    assert_eq!(exec.get_trade_grace_period(), 25);
}

/// Admin can set grace period to 0 (no delay).
#[test]
fn grace_period_can_be_zero() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_trade_grace_period(&0);
    assert_eq!(exec.get_trade_grace_period(), 0);
}

/// Queue a trade and cancel it within the grace period.
#[test]
fn cancel_queued_trade_within_grace_period_succeeds() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let user = funded_user(&env, &token);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let queued_id = exec.queue_copy_trade(&user, &token, &AMOUNT, &None);
    assert!(queued_id > 0);

    env.ledger().with_mut(|l| l.sequence_number = 5);
    let result = exec.try_cancel_queued_trade(&user, &queued_id);
    assert!(result.is_ok(), "cancellation within grace period should succeed");
}

/// Cancel after the grace period has elapsed should fail.
#[test]
fn cancel_after_grace_period_elapsed_fails() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let user = funded_user(&env, &token);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let queued_id = exec.queue_copy_trade(&user, &token, &AMOUNT, &None);

/// Execute queued trades after grace period — only eligible ones execute.
#[test]
fn execute_queued_trades_after_grace_period() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let user = funded_user(&env, &token);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let _queued_id = exec.queue_copy_trade(&user, &token, &AMOUNT, &None);

    // Not yet eligible
    env.ledger().with_mut(|l| l.sequence_number = 5);
    let count = exec.execute_queued_trades();
    assert_eq!(count, 0, "no trades before grace period elapses");

    // Eligible now
    env.ledger().with_mut(|l| l.sequence_number = 15);
    let count = exec.execute_queued_trades();
    assert_eq!(count, 1, "trade executes after grace period");
}

/// Multiple queued trades — only those past the grace period execute.
#[test]
fn multiple_queued_trades_partial_execution() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let user1 = funded_user(&env, &token);
    let user2 = funded_user(&env, &token);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    exec.queue_copy_trade(&user1, &token, &AMOUNT, &None);

    env.ledger().with_mut(|l| l.sequence_number = 8);
    exec.queue_copy_trade(&user2, &token, &AMOUNT, &None);

    // Only trade 1 eligible (12-0 >= 10)
    env.ledger().with_mut(|l| l.sequence_number = 12);
    assert_eq!(exec.execute_queued_trades(), 1);

    // Trade 2 eligible now
    env.ledger().with_mut(|l| l.sequence_number = 20);
    assert_eq!(exec.execute_queued_trades(), 1);
}

/// Grace period of 0 means trades are immediately eligible.
#[test]
fn zero_grace_period_executes_immediately() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let user = funded_user(&env, &token);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    exec.set_trade_grace_period(&0);
    env.ledger().with_mut(|l| l.sequence_number = 5);
    let _queued_id = exec.queue_copy_trade(&user, &token, &AMOUNT, &None);
    assert_eq!(exec.execute_queued_trades(), 1, "immediate with grace 0");
}

/// Cancelled trade is not executed.
#[test]
fn cancelled_trade_not_executed() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let user = funded_user(&env, &token);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let queued_id = exec.queue_copy_trade(&user, &token, &AMOUNT, &None);

    env.ledger().with_mut(|l| l.sequence_number = 5);
    exec.cancel_queued_trade(&user, &queued_id);

    env.ledger().with_mut(|l| l.sequence_number = 20);
    assert_eq!(exec.execute_queued_trades(), 0, "cancelled trade not executed");
}

    env.ledger().with_mut(|l| l.sequence_number = 15);
    let result = exec.try_cancel_queued_trade(&user, &queued_id);
    assert_eq!(result, Err(Ok(ContractError::GracePeriodExpired)));
}

/// A non-owner cannot cancel a queued trade.
#[test]
fn non_owner_cannot_cancel_queued_trade() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let user = funded_user(&env, &token);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let queued_id = exec.queue_copy_trade(&user, &token, &AMOUNT, &None);

    let impostor = Address::generate(&env);
    let result = exec.try_cancel_queued_trade(&impostor, &queued_id);
    assert_eq!(result, Err(Ok(ContractError::NotTradeOwner)));
}

/// Cancelling a non-existent queued trade returns QueuedTradeNotFound.
#[test]
fn cancel_nonexistent_queued_trade_fails() {
    let (env, exec_id, _) = setup();
    let user = Address::generate(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let result = exec.try_cancel_queued_trade(&user, &999u64);
    assert_eq!(result, Err(Ok(ContractError::QueuedTradeNotFound)));
}

