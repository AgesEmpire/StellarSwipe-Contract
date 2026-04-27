//! Dollar-cost averaging (DCA) copy trading (Issue #360).
//!
//! A DCA plan splits a total trade amount into `num_intervals` equal parts,
//! executing one part every `interval_ledgers` ledgers.  A keeper network
//! calls `execute_dca_interval` on schedule.  If the signal expires before
//! all intervals complete the plan is automatically cancelled.

use soroban_sdk::{contracttype, Address, Env};

use crate::errors::ContractError;
use crate::StorageKey;

// ── Plan struct ───────────────────────────────────────────────────────────────

/// Persisted state for an active DCA plan.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DCAPlan {
    /// Amount to trade per interval.
    pub amount_per_interval: i128,
    /// Number of intervals still to execute (counts down to 0).
    pub remaining_intervals: u32,
    /// Total intervals originally requested (for event reporting).
    pub total_intervals: u32,
    /// Ledger number after which the next interval may execute.
    pub next_interval_ledger: u32,
    /// Spacing between intervals in ledgers.
    pub interval_ledgers: u32,
    /// Ledger number after which the signal is considered expired (0 = no expiry).
    pub signal_expiry_ledger: u32,
    /// Running total of amount already executed (for `DCAPlanCompleted` event).
    pub executed_amount: i128,
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn plan_key(user: &Address, signal_id: u64) -> StorageKey {
    StorageKey::DCAPlan(user.clone(), signal_id)
}

pub fn load_plan(env: &Env, user: &Address, signal_id: u64) -> Result<DCAPlan, ContractError> {
    env.storage()
        .persistent()
        .get(&plan_key(user, signal_id))
        .ok_or(ContractError::DCAPlanNotFound)
}

fn save_plan(env: &Env, user: &Address, signal_id: u64, plan: &DCAPlan) {
    env.storage()
        .persistent()
        .set(&plan_key(user, signal_id), plan);
}

fn remove_plan(env: &Env, user: &Address, signal_id: u64) {
    env.storage()
        .persistent()
        .remove(&plan_key(user, signal_id));
}

// ── Public entrypoints ────────────────────────────────────────────────────────

/// Create a new DCA plan for `(user, signal_id)`.
///
/// - `total_amount` is split evenly across `num_intervals`.
/// - `interval_ledgers` is the minimum ledger gap between executions.
/// - `signal_expiry_ledger` is the ledger after which the signal is expired
///   (pass `0` for no expiry).
/// - The first interval is immediately due (next_interval_ledger = current).
pub fn execute_dca_copy_trade(
    env: &Env,
    user: &Address,
    signal_id: u64,
    total_amount: i128,
    num_intervals: u32,
    interval_ledgers: u32,
    signal_expiry_ledger: u32,
) -> Result<(), ContractError> {
    if total_amount <= 0 || num_intervals == 0 || interval_ledgers == 0 {
        return Err(ContractError::InvalidAmount);
    }

    // Reject if a plan already exists.
    if env
        .storage()
        .persistent()
        .has(&plan_key(user, signal_id))
    {
        return Err(ContractError::DCAPlanAlreadyExists);
    }

    // Check signal not already expired.
    let current_ledger = env.ledger().sequence();
    if signal_expiry_ledger > 0 && current_ledger >= signal_expiry_ledger {
        return Err(ContractError::SignalExpired);
    }

    let amount_per_interval = total_amount / num_intervals as i128;
    if amount_per_interval <= 0 {
        return Err(ContractError::InvalidAmount);
    }

    let plan = DCAPlan {
        amount_per_interval,
        remaining_intervals: num_intervals,
        total_intervals: num_intervals,
        next_interval_ledger: current_ledger, // first interval is immediately due
        interval_ledgers,
        signal_expiry_ledger,
        executed_amount: 0,
    };

    save_plan(env, user, signal_id, &plan);
    Ok(())
}

/// Execute the next DCA interval for `(user, signal_id)`.
///
/// Called by the keeper network.  Validates timing and signal expiry, then
/// executes the interval amount via the provided `execute_fn` callback (which
/// wraps the actual copy-trade logic so this module stays testable without
/// cross-contract calls).
///
/// Returns `Ok(true)` when the plan is now complete, `Ok(false)` otherwise.
pub fn execute_dca_interval<F>(
    env: &Env,
    user: &Address,
    signal_id: u64,
    execute_fn: F,
) -> Result<bool, ContractError>
where
    F: FnOnce(i128) -> Result<(), ContractError>,
{
    let mut plan = load_plan(env, user, signal_id)?;

    let current_ledger = env.ledger().sequence();

    // Cancel if signal expired.
    if plan.signal_expiry_ledger > 0 && current_ledger >= plan.signal_expiry_ledger {
        let intervals_completed = plan.total_intervals - plan.remaining_intervals;
        remove_plan(env, user, signal_id);
        shared::events::emit_dca_plan_cancelled(
            env,
            shared::events::EvtDCAPlanCancelled {
                schema_version: shared::events::SCHEMA_VERSION,
                user: user.clone(),
                signal_id,
                intervals_completed,
                reason: 0, // signal_expired
            },
        );
        return Err(ContractError::SignalExpired);
    }

    // Enforce interval timing.
    if current_ledger < plan.next_interval_ledger {
        return Err(ContractError::IntervalNotDue);
    }

    // Execute the trade for this interval.
    execute_fn(plan.amount_per_interval)?;

    plan.executed_amount += plan.amount_per_interval;
    plan.remaining_intervals -= 1;
    let interval_index = plan.total_intervals - plan.remaining_intervals; // 1-based

    shared::events::emit_dca_interval_executed(
        env,
        shared::events::EvtDCAIntervalExecuted {
            schema_version: shared::events::SCHEMA_VERSION,
            user: user.clone(),
            signal_id,
            interval_index,
            amount: plan.amount_per_interval,
            remaining_intervals: plan.remaining_intervals,
        },
    );

    if plan.remaining_intervals == 0 {
        let total_amount = plan.executed_amount;
        remove_plan(env, user, signal_id);
        shared::events::emit_dca_plan_completed(
            env,
            shared::events::EvtDCAPlanCompleted {
                schema_version: shared::events::SCHEMA_VERSION,
                user: user.clone(),
                signal_id,
                total_amount,
            },
        );
        return Ok(true);
    }

    plan.next_interval_ledger = current_ledger + plan.interval_ledgers;
    save_plan(env, user, signal_id, &plan);
    Ok(false)
}

/// Manually cancel a DCA plan. Only the plan owner may cancel.
pub fn cancel_dca_plan(
    env: &Env,
    user: &Address,
    signal_id: u64,
) -> Result<(), ContractError> {
    let plan = load_plan(env, user, signal_id)?;
    let intervals_completed = plan.total_intervals - plan.remaining_intervals;
    remove_plan(env, user, signal_id);

    shared::events::emit_dca_plan_cancelled(
        env,
        shared::events::EvtDCAPlanCancelled {
            schema_version: shared::events::SCHEMA_VERSION,
            user: user.clone(),
            signal_id,
            intervals_completed,
            reason: 1, // manual
        },
    );
    Ok(())
}
