//! Auto-trade intents: deadline-bound, budget-capped, pausable per strategy,
//! and gated on signal freshness at execution time.
//!
//! Deadline semantics (#1362): an intent may be executed while
//! `ledger.timestamp() <= deadline`; at creation the deadline must be strictly
//! in the future (`deadline > now`).
//!
//! Budget semantics (#1363): `committed + pending + amount <= budget`, where
//! `pending` is reserved by outstanding intents, moved to `committed` on
//! settlement and released on cancellation.
//!
//! Pause semantics (#1364): only the strategy owner may pause/resume; a paused
//! strategy rejects intent creation and execution without affecting others.
//!
//! Freshness (#1365): the signal is re-read from storage at execution and must
//! be `Active` and unexpired; failures return before any settlement.
//!
//! Replay protection (#1360): an intent is identified by
//! `(user, strategy_id, signal_id, sequence)`. The first successful
//! execution stores its [`TradeResult`] in the same transaction as
//! settlement, so either both persist or neither does. Any retry of the same
//! intent returns that stored result without executing another trade.
//!
//! Strategy revision binding (#1361): every strategy carries a monotonic
//! revision that advances on each settings update. An intent must name the
//! revision it was authorized against; a stale revision is rejected so
//! edited risk limits or routes can never silently apply to an old intent.

use soroban_sdk::{contracttype, Address, Env};

use crate::errors::AutoTradeError;
use crate::storage::{self, SignalStatus};
use crate::TradeResult;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntentStatus {
    Pending,
    Executed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub id: u64,
    pub owner: Address,
    pub strategy_id: u64,
    pub signal_id: u64,
    pub amount: i128,
    pub deadline: u64,
    pub status: IntentStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StrategyState {
    pub budget: i128,
    pub committed: i128,
    pub pending: i128,
    pub paused: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentId {
    pub user: Address,
    pub strategy_id: u64,
    pub signal_id: u64,
    pub sequence: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentRecord {
    /// Strategy revision the intent executed under.
    pub revision: u64,
    pub executed_at: u64,
    pub result: TradeResult,
}

#[contracttype]
enum IntentKey {
    NextId,
    Intent(u64),
    Strategy(Address, u64),
    Record(IntentId),
    StrategyRevision(u64),
}

pub fn get_strategy(env: &Env, owner: &Address, strategy_id: u64) -> StrategyState {
    env.storage()
        .persistent()
        .get(&IntentKey::Strategy(owner.clone(), strategy_id))
        .unwrap_or_default()
}

fn put_strategy(env: &Env, owner: &Address, strategy_id: u64, s: &StrategyState) {
    env.storage()
        .persistent()
        .set(&IntentKey::Strategy(owner.clone(), strategy_id), s);
}

pub fn get_intent(env: &Env, id: u64) -> Option<Intent> {
    env.storage().persistent().get(&IntentKey::Intent(id))
}

fn put_intent(env: &Env, intent: &Intent) {
    env.storage()
        .persistent()
        .set(&IntentKey::Intent(intent.id), intent);
}

pub fn set_budget(
    env: &Env,
    owner: &Address,
    strategy_id: u64,
    budget: i128,
) -> Result<(), AutoTradeError> {
    owner.require_auth();
    if budget < 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    let mut s = get_strategy(env, owner, strategy_id);
    s.budget = budget;
    put_strategy(env, owner, strategy_id, &s);
    Ok(())
}

pub fn set_paused(env: &Env, owner: &Address, strategy_id: u64, paused: bool) {
    owner.require_auth();
    let mut s = get_strategy(env, owner, strategy_id);
    s.paused = paused;
    put_strategy(env, owner, strategy_id, &s);
}

/// Signal must exist, be `Active`, and `now <= expiry`.
pub fn check_signal_fresh(env: &Env, signal_id: u64) -> Result<(), AutoTradeError> {
    let signal = storage::get_signal(env, signal_id).ok_or(AutoTradeError::SignalNotFound)?;
    if storage::get_signal_status(env, signal_id) != SignalStatus::Active {
        return Err(AutoTradeError::SignalInactive);
    }
    if env.ledger().timestamp() > signal.expiry {
        return Err(AutoTradeError::SignalExpired);
    }
    Ok(())
}

pub fn create(
    env: &Env,
    owner: &Address,
    strategy_id: u64,
    signal_id: u64,
    amount: i128,
    deadline: u64,
) -> Result<u64, AutoTradeError> {
    owner.require_auth();
    if amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    if deadline <= env.ledger().timestamp() {
        return Err(AutoTradeError::InvalidDeadline);
    }
    let mut s = get_strategy(env, owner, strategy_id);
    if s.paused {
        return Err(AutoTradeError::StrategyPaused);
    }
    check_signal_fresh(env, signal_id)?;
    let total = s
        .committed
        .checked_add(s.pending)
        .and_then(|v| v.checked_add(amount))
        .ok_or(AutoTradeError::BudgetExceeded)?;
    if total > s.budget {
        return Err(AutoTradeError::BudgetExceeded);
    }
    s.pending += amount;
    put_strategy(env, owner, strategy_id, &s);

    let id: u64 = env.storage().instance().get(&IntentKey::NextId).unwrap_or(0);
    env.storage().instance().set(&IntentKey::NextId, &(id + 1));
    put_intent(
        env,
        &Intent {
            id,
            owner: owner.clone(),
            strategy_id,
            signal_id,
            amount,
            deadline,
            status: IntentStatus::Pending,
        },
    );
    Ok(id)
}

/// Pre-execution validation. Returns the intent if executable.
pub fn validate_execution(env: &Env, id: u64) -> Result<Intent, AutoTradeError> {
    let intent = get_intent(env, id).ok_or(AutoTradeError::IntentNotFound)?;
    if intent.status != IntentStatus::Pending {
        return Err(AutoTradeError::IntentNotPending);
    }
    if env.ledger().timestamp() > intent.deadline {
        return Err(AutoTradeError::IntentExpired);
    }
    if get_strategy(env, &intent.owner, intent.strategy_id).paused {
        return Err(AutoTradeError::StrategyPaused);
    }
    check_signal_fresh(env, intent.signal_id)?;
    Ok(intent)
}

/// Settle a successfully executed intent: pending -> committed.
pub fn settle(env: &Env, mut intent: Intent) {
    let mut s = get_strategy(env, &intent.owner, intent.strategy_id);
    s.pending -= intent.amount;
    s.committed += intent.amount;
    put_strategy(env, &intent.owner, intent.strategy_id, &s);
    intent.status = IntentStatus::Executed;
    put_intent(env, &intent);
}

/// Cancel a pending intent (owner only), releasing its reservation.
pub fn cancel(env: &Env, owner: &Address, id: u64) -> Result<(), AutoTradeError> {
    owner.require_auth();
    let mut intent = get_intent(env, id).ok_or(AutoTradeError::IntentNotFound)?;
    if intent.owner != *owner {
        return Err(AutoTradeError::Unauthorized);
    }
    if intent.status != IntentStatus::Pending {
        return Err(AutoTradeError::IntentNotPending);
    }
    let mut s = get_strategy(env, owner, intent.strategy_id);
    s.pending -= intent.amount;
    put_strategy(env, owner, intent.strategy_id, &s);
    intent.status = IntentStatus::Cancelled;
    put_intent(env, &intent);
    Ok(())
}

/// Current revision of `strategy_id` (starts at 0).
pub fn strategy_revision(env: &Env, strategy_id: u64) -> u64 {
    env.storage()
        .persistent()
        .get(&IntentKey::StrategyRevision(strategy_id))
        .unwrap_or(0)
}

/// Advance the strategy revision after a settings update; returns the new value.
pub fn bump_strategy_revision(env: &Env, strategy_id: u64) -> u64 {
    let next = strategy_revision(env, strategy_id)
        .checked_add(1)
        .expect("strategy revision overflow");
    env.storage()
        .persistent()
        .set(&IntentKey::StrategyRevision(strategy_id), &next);
    next
}

pub fn get_record(env: &Env, id: &IntentId) -> Option<IntentRecord> {
    env.storage()
        .persistent()
        .get(&IntentKey::Record(id.clone()))
}

pub fn save_record(env: &Env, id: &IntentId, record: &IntentRecord) {
    env.storage()
        .persistent()
        .set(&IntentKey::Record(id.clone()), record);
}
}
