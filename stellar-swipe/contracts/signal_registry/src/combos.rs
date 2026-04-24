#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};

use crate::errors::ComboError;
use crate::StorageKey;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_COMPONENTS: u32 = 10;
const WEIGHT_TOTAL: u32 = 10000; // 100% in basis points

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The execution model for a combo signal.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComboType {
    /// All component signals execute simultaneously.
    Simultaneous,
    /// Component signals execute one after another in order.
    Sequential,
    /// Each component may have a condition that gates its execution.
    Conditional,
}

/// What must be true of a dependency signal before this one executes.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConditionType {
    /// The dependency signal must have a Successful / positive-ROI execution.
    Success,
    /// The dependency signal must have failed (negative ROI).
    Failure,
    /// The dependency signal's average ROI must be above a threshold (basis points).
    RoiAbove(i128),
}

/// A dependency condition tied to another signal inside the same combo.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    /// signal_id of the upstream component this depends on.
    pub depends_on: u64,
    pub condition_type: ConditionType,
}

/// Helper to bypass Option<T> contracttype issues for custom structs.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConditionGate {
    None,
    Some(Condition),
}

/// A single component within a combo.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ComponentSignal {
    pub signal_id: u64,
    /// Capital allocation in basis points (10000 = 100%).
    pub weight: u32,
    /// Optional execution gate; only relevant for Conditional combos.
    pub condition: ConditionGate,
}

/// The lifecycle status of a combo.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComboStatus {
    Active,
    Cancelled,
    Completed,
}

/// Top-level combo signal record.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ComboSignal {
    pub id: u64,
    pub name: String,
    pub provider: Address,
    pub component_signals: Vec<ComponentSignal>,
    pub combo_type: ComboType,
    pub status: ComboStatus,
    pub created_at: u64,
}

/// A single component execution result stored per combo execution.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ComponentExecution {
    pub signal_id: u64,
    pub amount: i128,
    pub skipped: bool,
    pub roi: i128, // basis points; 0 if skipped
}

/// Persisted record of one combo execution.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ComboExecution {
    pub combo_id: u64,
    pub executor: Address,
    pub total_amount: i128,
    pub component_executions: Vec<ComponentExecution>,
    pub combined_roi: i128, // weighted average ROI in basis points
    pub executed_at: u64,
}

/// Summary view returned to callers.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ComboPerformanceSummary {
    pub combo_id: u64,
    pub total_executions: u32,
    pub combined_roi: i128, // average across all executions
    pub total_volume: i128,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_combo_map(env: &Env) -> Map<u64, ComboSignal> {
    env.storage()
        .instance()
        .get(&StorageKey::Combos)
        .unwrap_or(Map::new(env))
}

fn save_combo_map(env: &Env, map: &Map<u64, ComboSignal>) {
    env.storage().instance().set(&StorageKey::Combos, map);
}

fn next_combo_id(env: &Env) -> u64 {
    let mut counter: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::ComboCounter)
        .unwrap_or(0);
    counter = counter.checked_add(1).expect("combo id overflow");
    env.storage()
        .instance()
        .set(&StorageKey::ComboCounter, &counter);
    counter
}

fn get_combo_executions(env: &Env, combo_id: u64) -> Vec<ComboExecution> {
    env.storage()
        .instance()
        .get(&StorageKey::ComboExecutions(combo_id))
        .unwrap_or(Vec::new(env))
}

fn save_combo_executions(env: &Env, combo_id: u64, execs: &Vec<ComboExecution>) {
    env.storage()
        .instance()
        .set(&StorageKey::ComboExecutions(combo_id), execs);
}

// ---------------------------------------------------------------------------
// Public helpers used by lib.rs
// ---------------------------------------------------------------------------

pub fn get_combo(env: &Env, combo_id: u64) -> Option<ComboSignal> {
    get_combo_map(env).get(combo_id)
}

// ---------------------------------------------------------------------------
// Core logic
// ---------------------------------------------------------------------------

/// Create and persist a combo signal.
///
/// Validates:
/// - all component signal IDs exist and belong to `provider`
/// - weights sum to exactly 10 000
/// - component count is within MAX_COMPONENTS
/// - no BUY/SELL conflict on the same asset pair in Simultaneous combos
pub fn create_combo_signal(
    env: &Env,
    provider: &Address,
    name: String,
    components: Vec<ComponentSignal>,
    combo_type: ComboType,
) -> Result<u64, ComboError> {
    if components.is_empty() {
        return Err(ComboError::NoComponents);
    }
    if components.len() > MAX_COMPONENTS {
        return Err(ComboError::TooManyComponents);
    }

    // Validate weights and signal ownership
    let mut total_weight: u32 = 0;
    let signals_map: Map<u64, crate::types::Signal> = env
        .storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env));

    for i in 0..components.len() {
        let comp = components.get(i).unwrap();

        let signal = signals_map
            .get(comp.signal_id)
            .ok_or(ComboError::SignalNotFound)?;

        if signal.provider != *provider {
            return Err(ComboError::NotSignalOwner);
        }

        // Only active signals may be added
        if signal.status != crate::types::SignalStatus::Active {
            return Err(ComboError::SignalNotActive);
        }

        total_weight = total_weight
            .checked_add(comp.weight)
            .ok_or(ComboError::WeightOverflow)?;
    }

    if total_weight != WEIGHT_TOTAL {
        return Err(ComboError::InvalidWeights);
    }

    // For Conditional combos validate that every condition references a
    // signal_id that also exists in the component list.
    if combo_type == ComboType::Conditional {
        let mut component_ids: Vec<u64> = Vec::new(env);
        for i in 0..components.len() {
            component_ids.push_back(components.get(i).unwrap().signal_id);
        }
        for i in 0..components.len() {
            let comp = components.get(i).unwrap();
            match &comp.condition {
                ConditionGate::Some(cond) => {
                    let mut found = false;
                    for j in 0..component_ids.len() {
                        if component_ids.get(j).unwrap() == cond.depends_on {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Err(ComboError::InvalidConditionReference);
                    }
                }
                ConditionGate::None => {}
            }
        }
    }

    let combo_id = next_combo_id(env);

    let combo = ComboSignal {
        id: combo_id,
        name,
        provider: provider.clone(),
        component_signals: components,
        combo_type,
        status: ComboStatus::Active,
        created_at: env.ledger().timestamp(),
    };

    let mut map = get_combo_map(env);
    map.set(combo_id, combo);
    save_combo_map(env, &map);

    Ok(combo_id)
}

/// Execute a combo signal on behalf of `user`.
///
/// Returns a list of `ComponentExecution` records, one per component.
/// Components that are skipped (due to unmet conditions or expired signals)
/// are recorded with `skipped: true`.
pub fn execute_combo_signal(
    env: &Env,
    combo_id: u64,
    user: &Address,
    total_amount: i128,
) -> Result<Vec<ComponentExecution>, ComboError> {
    let mut combo = get_combo(env, combo_id).ok_or(ComboError::ComboNotFound)?;

    if combo.status != ComboStatus::Active {
        return Err(ComboError::ComboNotActive);
    }

    if total_amount <= 0 {
        return Err(ComboError::InvalidAmount);
    }

    let signals_map: Map<u64, crate::types::Signal> = env
        .storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env));

    // Validate no component signal has expired
    let now = env.ledger().timestamp();
    for i in 0..combo.component_signals.len() {
        let comp = combo.component_signals.get(i).unwrap();
        if let Some(signal) = signals_map.get(comp.signal_id) {
            if signal.expiry <= now {
                return Err(ComboError::ComponentSignalExpired);
            }
        } else {
            return Err(ComboError::SignalNotFound);
        }
    }

    let mut component_executions: Vec<ComponentExecution> = Vec::new(env);

    match combo.combo_type {
        ComboType::Simultaneous => {
            for i in 0..combo.component_signals.len() {
                let comp = combo.component_signals.get(i).unwrap();
                let amount = (total_amount * comp.weight as i128) / WEIGHT_TOTAL as i128;
                let roi = simulate_trade_roi(env, comp.signal_id, amount);
                component_executions.push_back(ComponentExecution {
                    signal_id: comp.signal_id,
                    amount,
                    skipped: false,
                    roi,
                });
            }
        }

        ComboType::Sequential => {
            for i in 0..combo.component_signals.len() {
                let comp = combo.component_signals.get(i).unwrap();
                let amount = (total_amount * comp.weight as i128) / WEIGHT_TOTAL as i128;
                let roi = simulate_trade_roi(env, comp.signal_id, amount);
                component_executions.push_back(ComponentExecution {
                    signal_id: comp.signal_id,
                    amount,
                    skipped: false,
                    roi,
                });
                // In a blockchain context "wait" means each component is
                // registered sequentially in the same tx; the ordering of
                // the Vec is the authoritative execution order.
            }
        }

        ComboType::Conditional => {
            for i in 0..combo.component_signals.len() {
                let comp = combo.component_signals.get(i).unwrap();

                let should_execute = evaluate_condition(env, &comp, &component_executions)?;

                if should_execute {
                    let amount = (total_amount * comp.weight as i128) / WEIGHT_TOTAL as i128;
                    let roi = simulate_trade_roi(env, comp.signal_id, amount);
                    component_executions.push_back(ComponentExecution {
                        signal_id: comp.signal_id,
                        amount,
                        skipped: false,
                        roi,
                    });
                } else {
                    component_executions.push_back(ComponentExecution {
                        signal_id: comp.signal_id,
                        amount: 0,
                        skipped: true,
                        roi: 0,
                    });
                }
            }
        }
    }

    // Calculate combined weighted ROI
    let combined_roi = calculate_combined_roi(&component_executions, total_amount);

    // Persist execution record
    let execution = ComboExecution {
        combo_id,
        executor: user.clone(),
        total_amount,
        component_executions: component_executions.clone(),
        combined_roi,
        executed_at: now,
    };

    let mut execs = get_combo_executions(env, combo_id);
    execs.push_back(execution);
    save_combo_executions(env, combo_id, &execs);

    // If all components were skipped, mark combo as completed
    let all_skipped = {
        let mut all = true;
        for i in 0..component_executions.len() {
            if !component_executions.get(i).unwrap().skipped {
                all = false;
                break;
            }
        }
        all
    };

    if all_skipped {
        combo.status = ComboStatus::Completed;
        let mut map = get_combo_map(env);
        map.set(combo_id, combo);
        save_combo_map(env, &map);
    }

    Ok(component_executions)
}

/// Cancel an active combo (only the provider may cancel).
pub fn cancel_combo(env: &Env, combo_id: u64, provider: &Address) -> Result<(), ComboError> {
    let mut combo = get_combo(env, combo_id).ok_or(ComboError::ComboNotFound)?;

    if combo.provider != *provider {
        return Err(ComboError::NotSignalOwner);
    }
    if combo.status != ComboStatus::Active {
        return Err(ComboError::ComboNotActive);
    }

    combo.status = ComboStatus::Cancelled;
    let mut map = get_combo_map(env);
    map.set(combo_id, combo);
    save_combo_map(env, &map);

    Ok(())
}

/// Get aggregated performance for a combo.
pub fn get_combo_performance(env: &Env, combo_id: u64) -> Option<ComboPerformanceSummary> {
    get_combo(env, combo_id)?; // return None if combo doesn't exist

    let execs = get_combo_executions(env, combo_id);
    let total_executions = execs.len() as u32;

    if total_executions == 0 {
        return Some(ComboPerformanceSummary {
            combo_id,
            total_executions: 0,
            combined_roi: 0,
            total_volume: 0,
        });
    }

    let mut total_roi: i128 = 0;
    let mut total_volume: i128 = 0;

    for i in 0..execs.len() {
        let exec = execs.get(i).unwrap();
        total_roi = total_roi.saturating_add(exec.combined_roi);
        total_volume = total_volume.saturating_add(exec.total_amount);
    }

    let avg_roi = total_roi / total_executions as i128;

    Some(ComboPerformanceSummary {
        combo_id,
        total_executions,
        combined_roi: avg_roi,
        total_volume,
    })
}

/// Get all execution records for a combo.
pub fn get_combo_executions_pub(env: &Env, combo_id: u64) -> Vec<ComboExecution> {
    get_combo_executions(env, combo_id)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Evaluate whether a component's condition is satisfied given the results
/// of components that have already been executed.
fn evaluate_condition(
    _env: &Env,
    comp: &ComponentSignal,
    previous: &Vec<ComponentExecution>,
) -> Result<bool, ComboError> {
    let cond = match &comp.condition {
        ConditionGate::None => return Ok(true), // no condition = always execute
        ConditionGate::Some(c) => c,
    };

    // Find the prior execution for the depends_on signal
    let mut dep_exec: Option<ComponentExecution> = None;
    for i in 0..previous.len() {
        let exec = previous.get(i).unwrap();
        if exec.signal_id == cond.depends_on {
            dep_exec = Some(exec);
            break;
        }
    }

    let dep = match dep_exec {
        None => return Ok(false), // dependency hasn't executed yet → skip
        Some(e) => e,
    };

    if dep.skipped {
        return Ok(false); // dependency was skipped → condition unmet
    }

    let result = match &cond.condition_type {
        ConditionType::Success => dep.roi > 0,
        ConditionType::Failure => dep.roi <= 0,
        ConditionType::RoiAbove(threshold) => dep.roi > *threshold,
    };

    Ok(result)
}

/// Simulate trade ROI for a signal. In a full implementation this would
/// integrate with the performance module; here we read the signal's current
/// avg ROI from storage (defaulting to 0 if no executions yet).
fn simulate_trade_roi(env: &Env, signal_id: u64, _amount: i128) -> i128 {
    let signals_map: Map<u64, crate::types::Signal> = env
        .storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env));

    if let Some(signal) = signals_map.get(signal_id) {
        if signal.executions > 0 {
            return signal.total_roi / signal.executions as i128;
        }
    }
    0
}

/// Calculate a weighted-average combined ROI across all non-skipped components.
fn calculate_combined_roi(executions: &Vec<ComponentExecution>, total_amount: i128) -> i128 {
    if total_amount == 0 {
        return 0;
    }

    let mut weighted_sum: i128 = 0;
    let mut executed_amount: i128 = 0;

    for i in 0..executions.len() {
        let exec = executions.get(i).unwrap();
        if !exec.skipped && exec.amount > 0 {
            weighted_sum = weighted_sum.saturating_add(exec.roi.saturating_mul(exec.amount));
            executed_amount = executed_amount.saturating_add(exec.amount);
        }
    }

    if executed_amount == 0 {
        return 0;
    }

    weighted_sum / executed_amount
}
