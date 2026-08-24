#![cfg(test)]
//! Unit tests for the dead-letter queue feature (Issue #657).
//!
//! Covers:
//! - Default max retry count
//! - Admin can configure max retry count
//! - Failed trade is retried before dead-lettering
//! - Trade is dead-lettered after max retries
//! - `get_dead_letter_trades` returns the correct records
//! - Admin can requeue a dead-lettered trade
//! - Admin can discard a dead-lettered trade
//! - Successful trade is not dead-lettered

use crate::{
    errors::ContractError, FailedTrade, TradeExecutorContract, TradeExecutorContractClient,
    DEFAULT_MAX_RETRY_COUNT,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger as _},
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
    (env, exec_id, admin)
}

/// Queue a trade for a user that has NOT been funded — guaranteed to fail on execution.
fn queue_failing_trade(env: &Env, exec_id: &Address, token: &Address) -> (Address, u64) {
    let user = Address::generate(env); // no mint → always InsufficientBalance
    let exec = TradeExecutorContractClient::new(env, exec_id);
    let queued_id = exec.queue_copy_trade(&user, token, &AMOUNT, &None);
    (user, queued_id)
}

/// Advance ledger past the grace period so queued trades become eligible.
fn advance_past_grace(env: &Env) {
    env.ledger().with_mut(|l| l.sequence_number = 20);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Default max retry count is DEFAULT_MAX_RETRY_COUNT.
#[test]
fn default_max_retry_count_is_3() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    assert_eq!(exec.get_max_retry_count(), DEFAULT_MAX_RETRY_COUNT);
}

/// Admin can configure the max retry count.
#[test]
fn admin_can_set_max_retry_count() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_max_retry_count(&5);
    assert_eq!(exec.get_max_retry_count(), 5);
}

/// With max_retries = 3, a failing trade is retried twice before dead-lettering
/// (3 total attempts: attempt 1 → retry_count 1, attempt 2 → retry_count 2,
///  attempt 3 → retry_count 3 >= max → dead-letter).
#[test]
fn failed_trade_dead_lettered_after_max_retries() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let (user, _queued_id) = queue_failing_trade(&env, &exec_id, &token);
    advance_past_grace(&env);

    // First two calls: trade is retried (not yet dead-lettered).
    exec.execute_queued_trades();
    let dl_after_1 = exec.get_dead_letter_trades(&user);
    assert_eq!(
        dl_after_1.len(),
        0,
        "should not be dead-lettered after attempt 1"
    );

    exec.execute_queued_trades();
    let dl_after_2 = exec.get_dead_letter_trades(&user);
    assert_eq!(
        dl_after_2.len(),
        0,
        "should not be dead-lettered after attempt 2"
    );

    // Third call: max retries reached → dead-lettered.
    exec.execute_queued_trades();
    let dl_after_3 = exec.get_dead_letter_trades(&user);
    assert_eq!(
        dl_after_3.len(),
        1,
        "should be dead-lettered after attempt 3"
    );
}

/// `get_dead_letter_trades` returns the correct FailedTrade fields.
#[test]
fn dead_letter_trade_has_correct_fields() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let (user, _) = queue_failing_trade(&env, &exec_id, &token);
    advance_past_grace(&env);

    // Exhaust all retries.
    for _ in 0..DEFAULT_MAX_RETRY_COUNT {
        exec.execute_queued_trades();
    }

    let dl: Vec<FailedTrade> = exec.get_dead_letter_trades(&user);
    assert_eq!(dl.len(), 1);
    let ft = dl.get(0).unwrap();
    assert_eq!(ft.user, user);
    assert_eq!(ft.token, token);
    assert_eq!(ft.amount, AMOUNT);
    assert_eq!(ft.retry_count, DEFAULT_MAX_RETRY_COUNT);
    assert!(
        ft.failure_code > 0,
        "failure_code should be a non-zero error discriminant"
    );
}

/// A successful trade is never dead-lettered.
#[test]
fn successful_trade_not_dead_lettered() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    // Fund the user so the trade succeeds.
    let user = Address::generate(&env);
    soroban_sdk::token::StellarAssetClient::new(&env, &token).mint(&user, &(AMOUNT * 10));

    env.ledger().with_mut(|l| l.sequence_number = 0);
    exec.queue_copy_trade(&user, &token, &AMOUNT, &None);
    advance_past_grace(&env);

    let count = exec.execute_queued_trades();
    assert_eq!(count, 1, "trade should succeed");

    let dl = exec.get_dead_letter_trades(&user);
    assert_eq!(
        dl.len(),
        0,
        "successful trade must not appear in dead-letter queue"
    );
}

/// `get_dead_letter_trades` returns empty vec for a user with no failures.
#[test]
fn empty_dead_letter_for_clean_user() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let user = Address::generate(&env);
    let dl = exec.get_dead_letter_trades(&user);
    assert_eq!(dl.len(), 0);
}

/// Admin can requeue a dead-lettered trade; it re-enters the normal queue.
#[test]
fn admin_can_requeue_dead_lettered_trade() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let (user, queued_id) = queue_failing_trade(&env, &exec_id, &token);
    advance_past_grace(&env);

    for _ in 0..DEFAULT_MAX_RETRY_COUNT {
        exec.execute_queued_trades();
    }
    assert_eq!(exec.get_dead_letter_trades(&user).len(), 1);

    // Requeue — advances grace period too.
    exec.requeue_dead_lettered_trade(&queued_id);

    // Dead-letter queue should now be empty for this user.
    assert_eq!(
        exec.get_dead_letter_trades(&user).len(),
        0,
        "after requeue, dead-letter should be cleared"
    );
}

/// Admin can permanently discard a dead-lettered trade.
#[test]
fn admin_can_discard_dead_lettered_trade() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let (user, queued_id) = queue_failing_trade(&env, &exec_id, &token);
    advance_past_grace(&env);

    for _ in 0..DEFAULT_MAX_RETRY_COUNT {
        exec.execute_queued_trades();
    }
    assert_eq!(exec.get_dead_letter_trades(&user).len(), 1);

    exec.discard_dead_lettered_trade(&queued_id);
    assert_eq!(
        exec.get_dead_letter_trades(&user).len(),
        0,
        "after discard, dead-letter should be cleared"
    );
}

/// Multiple users' dead-letter queues are isolated — one user's failures don't
/// appear in another's `get_dead_letter_trades` results.
#[test]
fn dead_letter_queues_are_per_user() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let (user_a, _) = queue_failing_trade(&env, &exec_id, &token);
    let (user_b, _) = queue_failing_trade(&env, &exec_id, &token);
    advance_past_grace(&env);

    // Exhaust retries for both users.
    for _ in 0..DEFAULT_MAX_RETRY_COUNT {
        exec.execute_queued_trades();
    }

    // Each user sees only their own dead-lettered trade.
    assert_eq!(exec.get_dead_letter_trades(&user_a).len(), 1);
    assert_eq!(exec.get_dead_letter_trades(&user_b).len(), 1);

    // User C (never traded) sees empty list.
    let user_c = Address::generate(&env);
    assert_eq!(exec.get_dead_letter_trades(&user_c).len(), 0);
}
