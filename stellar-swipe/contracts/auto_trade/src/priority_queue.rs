use soroban_sdk::{contracttype, Env, Vec};

/// Classification of an auto-trade trigger.  Higher priority values are
/// processed first within a batch.  Within the same priority, actions are
/// processed in FIFO order (by `enqueued_at`).
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerType {
    StopLoss,
    TakeProfit,
    Rebalance,
}

impl TriggerType {
    pub fn priority(self) -> u32 {
        match self {
            TriggerType::StopLoss => 3,
            TriggerType::TakeProfit => 2,
            TriggerType::Rebalance => 1,
        }
    }
}

/// A single auto-trade action waiting for execution.
#[contracttype]
#[derive(Clone, Debug)]
pub struct QueuedAction {
    /// Monotonically increasing identifier assigned at enqueue time.
    pub id: u64,
    /// What kind of trade this is (determines processing priority).
    pub trigger: TriggerType,
    /// The signal that triggered this action.
    pub signal_id: u64,
    /// Ledger timestamp when this action was enqueued (used for FIFO tiebreak).
    pub enqueued_at: u64,
}

#[contracttype]
enum QueueKey {
    Actions,
    NextId,
}

/// Add a new action to the priority queue.
///
/// The queue is kept in descending priority order.  Within the same priority
/// the action is appended after all existing same-priority entries (FIFO).
pub fn enqueue_action(env: &Env, trigger: TriggerType, signal_id: u64) -> u64 {
    let next_id: u64 = env
        .storage()
        .instance()
        .get(&QueueKey::NextId)
        .unwrap_or(0u64);

    let action = QueuedAction {
        id: next_id,
        trigger,
        signal_id,
        enqueued_at: env.ledger().timestamp(),
    };

    let queue: Vec<QueuedAction> = env
        .storage()
        .instance()
        .get(&QueueKey::Actions)
        .unwrap_or_else(|| Vec::new(env));

    // Build a new vec inserting `action` at the correct priority position.
    // Entries with strictly lower priority than `action` are shifted right;
    // same-priority entries that are already present come before it (FIFO).
    let mut sorted = Vec::new(env);
    let mut inserted = false;
    for i in 0..queue.len() {
        let existing = queue.get(i).unwrap();
        if !inserted && action.trigger.priority() > existing.trigger.priority() {
            sorted.push_back(action.clone());
            inserted = true;
        }
        sorted.push_back(existing);
    }
    if !inserted {
        sorted.push_back(action);
    }

    env.storage().instance().set(&QueueKey::Actions, &sorted);
    env.storage()
        .instance()
        .set(&QueueKey::NextId, &(next_id + 1));

    next_id
}

/// Remove and return up to `limit` actions from the front of the queue
/// (i.e. the highest-priority, earliest-enqueued actions).
pub fn drain_priority_batch(env: &Env, limit: u32) -> Vec<QueuedAction> {
    let queue: Vec<QueuedAction> = env
        .storage()
        .instance()
        .get(&QueueKey::Actions)
        .unwrap_or_else(|| Vec::new(env));

    let take = limit.min(queue.len());
    let mut batch = Vec::new(env);
    let mut remaining = Vec::new(env);

    for i in 0..queue.len() {
        let entry = queue.get(i).unwrap();
        if i < take {
            batch.push_back(entry);
        } else {
            remaining.push_back(entry);
        }
    }

    env.storage().instance().set(&QueueKey::Actions, &remaining);
    batch
}

/// Read-only snapshot of all pending actions (highest priority first).
///
/// Intended for off-chain observability; does not mutate state.
pub fn get_queue_snapshot(env: &Env) -> Vec<QueuedAction> {
    env.storage()
        .instance()
        .get(&QueueKey::Actions)
        .unwrap_or_else(|| Vec::new(env))
}
