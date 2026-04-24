use soroban_sdk::{contracttype, Address, Bytes, Env, Map, Vec};

use crate::proposals::{self, ProposalStatus, ProposalType};
use crate::{GovernanceError, StorageKey};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionType {
    TreasurySpend,
    ParameterChange,
    ContractUpgrade,
    EmergencyPause,
    Custom,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedAction {
    pub id: u64,
    pub action_type: ActionType,
    pub proposal_id: u64,
    pub execution_payload: Bytes,
    pub queued_at: u64,
    pub execution_available: u64,
    pub executed: bool,
    pub cancelled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Timelock {
    pub queued_actions: Vec<QueuedAction>,
    pub delay_config: Map<ActionType, u64>,
    pub min_delay: u64,
    pub max_delay: u64,
    pub guardian: Address,
    pub next_action_id: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelockAnalytics {
    pub total_queued: u32,
    pub total_executed: u32,
    pub total_cancelled: u32,
    pub avg_wait_time: u64,
    pub actions_by_type: Map<ActionType, u32>,
    pub cancellation_rate: u32,
}

pub fn initialize_timelock(
    env: &Env,
    min_delay: u64,
    max_delay: u64,
    guardian: Address,
) -> Result<Timelock, GovernanceError> {
    if min_delay < 3600 || max_delay > 7 * 86_400 || min_delay > max_delay {
        return Err(GovernanceError::InvalidTimelockConfig);
    }

    let mut delay_config = Map::new(env);
    delay_config.set(ActionType::TreasurySpend, 2 * 86_400);
    delay_config.set(ActionType::ParameterChange, 3 * 86_400);
    delay_config.set(ActionType::ContractUpgrade, 5 * 86_400);
    delay_config.set(ActionType::EmergencyPause, 0);
    delay_config.set(ActionType::Custom, 2 * 86_400);

    let timelock = Timelock {
        queued_actions: Vec::new(env),
        delay_config,
        min_delay,
        max_delay,
        guardian: guardian.clone(),
        next_action_id: 1,
    };

    env.storage()
        .instance()
        .set(&StorageKey::TimelockState, &timelock);
    env.storage().instance().set(&StorageKey::Guardian, &guardian);

    Ok(timelock)
}

pub fn get_timelock(env: &Env) -> Result<Timelock, GovernanceError> {
    env.storage()
        .instance()
        .get(&StorageKey::TimelockState)
        .ok_or(GovernanceError::TimelockNotInitialized)
}

pub fn put_timelock(env: &Env, timelock: &Timelock) {
    env.storage()
        .instance()
        .set(&StorageKey::TimelockState, timelock);
}

pub fn queue_action(env: &Env, proposal_id: u64) -> Result<u64, GovernanceError> {
    let proposal = proposals::get_proposal(env, proposal_id)?;
    if proposal.status != ProposalStatus::Succeeded {
        return Err(GovernanceError::ProposalNotApproved);
    }

    let mut timelock = get_timelock(env)?;
    let action_type = classify_proposal_action(&proposal.proposal_type);
    let delay = timelock
        .delay_config
        .get(action_type.clone())
        .unwrap_or(timelock.min_delay);

    let mut i = 0;
    while i < timelock.queued_actions.len() {
        let a = timelock.queued_actions.get(i).unwrap();
        if a.proposal_id == proposal_id && !a.executed && !a.cancelled {
            return Err(GovernanceError::InvalidCommitteeAction);
        }
        i += 1;
    }

    let now = env.ledger().timestamp();
    let id = timelock.next_action_id;
    timelock.queued_actions.push_back(QueuedAction {
        id,
        action_type,
        proposal_id,
        execution_payload: proposal.execution_payload,
        queued_at: now,
        execution_available: now.saturating_add(delay),
        executed: false,
        cancelled: false,
    });
    timelock.next_action_id = id.saturating_add(1);

    put_timelock(env, &timelock);
    Ok(id)
}

pub fn execute_queued_action(
    env: &Env,
    action_id: u64,
    executor: Address,
) -> Result<(), GovernanceError> {
    executor.require_auth();
    let mut timelock = get_timelock(env)?;

    let mut i = 0;
    while i < timelock.queued_actions.len() {
        let mut action = timelock.queued_actions.get(i).unwrap();
        if action.id == action_id {
            if action.executed || action.cancelled {
                return Err(GovernanceError::InvalidCommitteeAction);
            }
            if env.ledger().timestamp() < action.execution_available {
                return Err(GovernanceError::InvalidDuration);
            }

            proposals::execute_proposal_action_by_id(env, action.proposal_id)?;
            action.executed = true;
            timelock.queued_actions.set(i, action.clone());
            put_timelock(env, &timelock);
            proposals::mark_proposal_executed(env, action.proposal_id)?;
            return Ok(());
        }
        i += 1;
    }

    Err(GovernanceError::ActionNotFound)
}

pub fn cancel_queued_action(
    env: &Env,
    action_id: u64,
    canceller: Address,
) -> Result<(), GovernanceError> {
    canceller.require_auth();
    let mut timelock = get_timelock(env)?;
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    if canceller != timelock.guardian && canceller != admin {
        return Err(GovernanceError::Unauthorized);
    }

    let mut i = 0;
    while i < timelock.queued_actions.len() {
        let mut action = timelock.queued_actions.get(i).unwrap();
        if action.id == action_id {
            if action.executed || action.cancelled {
                return Err(GovernanceError::InvalidCommitteeAction);
            }
            action.cancelled = true;
            timelock.queued_actions.set(i, action.clone());
            put_timelock(env, &timelock);

            let mut proposal = proposals::get_proposal(env, action.proposal_id)?;
            proposal.status = ProposalStatus::Cancelled;
            proposals::put_proposal(env, &proposal)?;
            return Ok(());
        }
        i += 1;
    }

    Err(GovernanceError::ActionNotFound)
}

pub fn update_timelock_delay(
    env: &Env,
    action_type: ActionType,
    new_delay: u64,
) -> Result<(), GovernanceError> {
    let mut timelock = get_timelock(env)?;
    if new_delay < timelock.min_delay || new_delay > timelock.max_delay {
        return Err(GovernanceError::InvalidDuration);
    }
    timelock.delay_config.set(action_type, new_delay);
    put_timelock(env, &timelock);
    Ok(())
}

pub fn emergency_execute(
    env: &Env,
    action_id: u64,
    guardian: Address,
) -> Result<(), GovernanceError> {
    guardian.require_auth();
    let mut timelock = get_timelock(env)?;
    if guardian != timelock.guardian {
        return Err(GovernanceError::Unauthorized);
    }

    let mut i = 0;
    while i < timelock.queued_actions.len() {
        let mut action = timelock.queued_actions.get(i).unwrap();
        if action.id == action_id {
            if action.action_type != ActionType::EmergencyPause {
                return Err(GovernanceError::InvalidCommitteeAction);
            }
            if action.executed || action.cancelled {
                return Err(GovernanceError::InvalidCommitteeAction);
            }
            proposals::execute_proposal_action_by_id(env, action.proposal_id)?;
            action.executed = true;
            timelock.queued_actions.set(i, action.clone());
            put_timelock(env, &timelock);
            proposals::mark_proposal_executed(env, action.proposal_id)?;
            return Ok(());
        }
        i += 1;
    }

    Err(GovernanceError::ActionNotFound)
}

pub fn extend_execution_window(
    env: &Env,
    action_id: u64,
    extension_seconds: u64,
) -> Result<u64, GovernanceError> {
    if extension_seconds > 7 * 86_400 {
        return Err(GovernanceError::InvalidDuration);
    }

    let mut timelock = get_timelock(env)?;
    let mut i = 0;
    while i < timelock.queued_actions.len() {
        let mut action = timelock.queued_actions.get(i).unwrap();
        if action.id == action_id {
            if action.executed || action.cancelled {
                return Err(GovernanceError::InvalidCommitteeAction);
            }
            action.execution_available = action.execution_available.saturating_add(extension_seconds);
            let new_time = action.execution_available;
            timelock.queued_actions.set(i, action);
            put_timelock(env, &timelock);
            return Ok(new_time);
        }
        i += 1;
    }
    Err(GovernanceError::ActionNotFound)
}

pub fn execute_multiple_actions(
    env: &Env,
    action_ids: Vec<u64>,
    executor: Address,
) -> Result<Vec<u64>, GovernanceError> {
    executor.require_auth();
    if action_ids.len() > 10 {
        return Err(GovernanceError::InvalidAmount);
    }

    let mut executed = Vec::new(env);
    let mut i = 0;
    while i < action_ids.len() {
        let id = action_ids.get(i).unwrap();
        if execute_queued_action(env, id, executor.clone()).is_ok() {
            executed.push_back(id);
        }
        i += 1;
    }
    Ok(executed)
}

pub fn generate_timelock_analytics(env: &Env) -> Result<TimelockAnalytics, GovernanceError> {
    let timelock = get_timelock(env)?;

    let total_queued = timelock.queued_actions.len();
    let mut total_executed = 0u32;
    let mut total_cancelled = 0u32;
    let mut total_wait = 0u64;
    let mut wait_count = 0u32;
    let mut by_type: Map<ActionType, u32> = Map::new(env);

    let mut i = 0;
    while i < timelock.queued_actions.len() {
        let action = timelock.queued_actions.get(i).unwrap();
        let count = by_type.get(action.action_type.clone()).unwrap_or(0);
        by_type.set(action.action_type.clone(), count.saturating_add(1));

        if action.executed {
            total_executed = total_executed.saturating_add(1);
            total_wait = total_wait.saturating_add(action.execution_available.saturating_sub(action.queued_at));
            wait_count = wait_count.saturating_add(1);
        }
        if action.cancelled {
            total_cancelled = total_cancelled.saturating_add(1);
        }
        i += 1;
    }

    Ok(TimelockAnalytics {
        total_queued,
        total_executed,
        total_cancelled,
        avg_wait_time: if wait_count > 0 {
            total_wait / wait_count as u64
        } else {
            0
        },
        actions_by_type: by_type,
        cancellation_rate: if total_queued > 0 {
            (total_cancelled * 10_000) / total_queued
        } else {
            0
        },
    })
}

fn classify_proposal_action(proposal_type: &ProposalType) -> ActionType {
    match proposal_type {
        ProposalType::TreasurySpend(..) => ActionType::TreasurySpend,
        ProposalType::ParameterChange(..) => ActionType::ParameterChange,
        ProposalType::ContractUpgrade(..) => ActionType::ContractUpgrade,
        _ => ActionType::Custom,
    }
}
