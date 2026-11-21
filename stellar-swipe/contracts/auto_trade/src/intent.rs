//! Auto-trade execution intents (Issues #1360, #1361).
//!
//! * **Replay protection (#1360):** an intent is identified by
//!   `(user, strategy_id, signal_id, sequence)`. The first successful
//!   execution stores its [`TradeResult`] in the same transaction as
//!   settlement, so either both persist or neither does. Any retry of the same
//!   intent returns that stored result without executing another trade.
//! * **Strategy revision binding (#1361):** every strategy carries a monotonic
//!   revision that advances on each settings update. An intent must name the
//!   revision it was authorized against; a stale revision is rejected so
//!   edited risk limits or routes can never silently apply to an old intent.

use crate::TradeResult;
use soroban_sdk::{contracttype, Address, Env};

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
#[derive(Clone)]
enum IntentKey {
    Record(IntentId),
    StrategyRevision(u64),
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
