#![cfg(test)]
//! Unit tests for TTL-based dead-letter queue pruning (Issue #792).
//!
//! Covers:
//! - `prune_dead_letter_queue` removes nothing when no entries are stale
//! - A mix of fresh and stale entries: only stale entries are removed
//! - `max_entries` bounds a single call to a partial prune

use crate::{
    errors::ContractError, TradeExecutorContract, TradeExecutorContractClient,
    DEFAULT_MAX_RETRY_COUNT,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger as _},
    Address, Env,
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

/// Queue a failing trade, advance past the grace period, and exhaust all
/// retries at the current ledger so it becomes dead-lettered right now.
fn dead_letter_now(env: &Env, exec_id: &Address, token: &Address) -> Address {
    let exec = TradeExecutorContractClient::new(env, exec_id);
    let queued_at = env.ledger().sequence();
    let (user, _queued_id) = queue_failing_trade(env, exec_id, token);
    env.ledger()
        .with_mut(|l| l.sequence_number = queued_at + 20);
    for _ in 0..DEFAULT_MAX_RETRY_COUNT {
        exec.execute_queued_trades();
    }
    assert_eq!(exec.get_dead_letter_trades(&user).len(), 1);
    user
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Default retention is [`crate::DEFAULT_DEAD_LETTER_RETENTION_LEDGERS`].
#[test]
fn default_retention_matches_constant() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    assert_eq!(
        exec.get_dead_letter_retention(),
        crate::DEFAULT_DEAD_LETTER_RETENTION_LEDGERS
    );
}

/// Admin can configure the retention window.
#[test]
fn admin_can_set_dead_letter_retention() {
    let (env, exec_id, _) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_dead_letter_retention(&100);
    assert_eq!(exec.get_dead_letter_retention(), 100);
}

/// (a) No stale entries: pruning removes nothing and leaves the queue intact.
#[test]
fn prune_removes_nothing_when_no_stale_entries() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let user = dead_letter_now(&env, &exec_id, &token);

    // Retention is the default (large) window — nothing is stale yet.
    let removed = exec.prune_dead_letter_queue(&None, &10);
    assert_eq!(removed, 0);
    assert_eq!(exec.get_dead_letter_trades(&user).len(), 1);
}

/// (b) A mix of fresh and stale entries: only the stale entry is removed.
#[test]
fn prune_removes_only_stale_entries() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let user_a = dead_letter_now(&env, &exec_id, &token); // dead-lettered at ledger 20

    env.ledger().with_mut(|l| l.sequence_number = 35);
    let user_b = dead_letter_now(&env, &exec_id, &token); // dead-lettered at ledger 55

    exec.set_dead_letter_retention(&10);
    env.ledger().with_mut(|l| l.sequence_number = 60);
    // user_a: 60 - 20 = 40 >= 10 → stale. user_b: 60 - 55 = 5 < 10 → fresh.

    let removed = exec.prune_dead_letter_queue(&None, &10);
    assert_eq!(removed, 1);
    assert_eq!(
        exec.get_dead_letter_trades(&user_a).len(),
        0,
        "stale entry should be removed"
    );
    assert_eq!(
        exec.get_dead_letter_trades(&user_b).len(),
        1,
        "fresh entry should remain"
    );
}

/// (c) `max_entries` caps a single call to a partial prune when more entries
/// are stale than the requested cap.
#[test]
fn prune_respects_max_entries_cap() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let user_a = dead_letter_now(&env, &exec_id, &token);

    env.ledger().with_mut(|l| l.sequence_number = 25);
    let user_b = dead_letter_now(&env, &exec_id, &token);

    env.ledger().with_mut(|l| l.sequence_number = 50);
    let user_c = dead_letter_now(&env, &exec_id, &token);

    exec.set_dead_letter_retention(&5);
    env.ledger().with_mut(|l| l.sequence_number = 200);
    // All three are well past the 5-ledger retention window at this point.

    let removed = exec.prune_dead_letter_queue(&None, &2);
    assert_eq!(removed, 2, "only max_entries should be removed in one call");

    let remaining = exec.get_dead_letter_trades(&user_a).len()
        + exec.get_dead_letter_trades(&user_b).len()
        + exec.get_dead_letter_trades(&user_c).len();
    assert_eq!(remaining, 1, "one stale entry should remain uncapped");

    // A second call sweeps the rest.
    let removed_2 = exec.prune_dead_letter_queue(&None, &10);
    assert_eq!(removed_2, 1);
}

/// Pruning scoped to a single `user` only inspects that user's dead-letter queue.
#[test]
fn prune_scoped_to_single_user() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 0);
    let user_a = dead_letter_now(&env, &exec_id, &token);
    let user_b = dead_letter_now(&env, &exec_id, &token);

    exec.set_dead_letter_retention(&5);
    env.ledger().with_mut(|l| l.sequence_number = 100);

    let removed = exec.prune_dead_letter_queue(&Some(user_a.clone()), &10);
    assert_eq!(removed, 1);
    assert_eq!(exec.get_dead_letter_trades(&user_a).len(), 0);
    assert_eq!(
        exec.get_dead_letter_trades(&user_b).len(),
        1,
        "user_b's entry must be untouched by a user-scoped prune"
    );
}
