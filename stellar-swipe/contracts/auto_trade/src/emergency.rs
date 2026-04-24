use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};

use crate::errors::AutoTradeError;

// ── Storage Keys ────────────────────────────────────────────────────────────

#[contracttype]
pub enum EmergencyKey {
    PauseState,
    Admin,
    Recovery(u64),
    RecoveryCounter,
}

// ── Types ────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PauseType {
    /// All operations stopped
    FullPause,
    /// New deposits (InitiateTransfer) blocked
    DepositsOnly,
    /// New withdrawals (InitiateWithdrawal) blocked
    WithdrawalsOnly,
    /// Validator signing blocked
    ValidationOnly,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeOperation {
    InitiateTransfer,
    InitiateWithdrawal,
    ValidatorSign,
    Other,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PauseState {
    pub is_paused: bool,
    pub pause_type: PauseType,
    pub paused_at: u64,       // 0 = never paused
    pub paused_by: Address,   // last pauser (sentinel: contract address when not set)
    pub pause_reason: String,
    pub auto_unpause_at: u64, // 0 = no auto-unpause
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryCheck {
    pub name: String,
    pub completed: bool,
    pub verified_by: Address, // sentinel: contract address when not verified
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryChecklist {
    pub recovery_id: u64,
    pub initiated_by: Address,
    pub initiated_at: u64,
    pub checks: Vec<RecoveryCheck>,
    pub all_checks_complete: bool,
}

// ── Admin helpers ────────────────────────────────────────────────────────────

pub fn set_emergency_admin(env: &Env, admin: &Address) {
    env.storage()
        .persistent()
        .set(&EmergencyKey::Admin, admin);
}

pub fn get_emergency_admin(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&EmergencyKey::Admin)
}

fn is_admin(env: &Env, caller: &Address) -> bool {
    get_emergency_admin(env)
        .map(|a| a == *caller)
        .unwrap_or(false)
}

// ── Pause state helpers ──────────────────────────────────────────────────────

fn get_pause_state(env: &Env) -> PauseState {
    env.storage()
        .persistent()
        .get(&EmergencyKey::PauseState)
        .unwrap_or(PauseState {
            is_paused: false,
            pause_type: PauseType::FullPause,
            paused_at: 0,
            paused_by: env.current_contract_address(),
            pause_reason: String::from_str(env, ""),
            auto_unpause_at: 0,
        })
}

fn save_pause_state(env: &Env, state: &PauseState) {
    env.storage()
        .persistent()
        .set(&EmergencyKey::PauseState, state);
}

// ── Public: Emergency Pause ──────────────────────────────────────────────────

/// Pause the bridge. Caller must be the emergency admin.
/// First pause wins — subsequent calls while already paused are logged but ignored.
pub fn emergency_pause(
    env: &Env,
    caller: &Address,
    pause_type: PauseType,
    reason: String,
) -> Result<(), AutoTradeError> {
    if !is_admin(env, caller) {
        return Err(AutoTradeError::Unauthorized);
    }

    let mut state = get_pause_state(env);

    if state.is_paused {
        // First pause wins; emit a secondary-pause-attempt event and return ok
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "pause_attempt_ignored"), caller.clone()),
            reason,
        );
        return Ok(());
    }

    state.is_paused = true;
    state.pause_type = pause_type.clone();
    state.paused_at = env.ledger().timestamp();
    state.paused_by = caller.clone();
    state.pause_reason = reason.clone();
    state.auto_unpause_at = 0;
    save_pause_state(env, &state);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "bridge_paused"),
            caller.clone(),
            pause_type,
        ),
        reason,
    );

    Ok(())
}

// ── Public: Enforce Pause ────────────────────────────────────────────────────

/// Call at the top of any operation that should respect pause state.
pub fn enforce_pause(env: &Env, op: &BridgeOperation) -> Result<(), AutoTradeError> {
    let state = get_pause_state(env);

    if !state.is_paused {
        return Ok(());
    }

    // Check auto-unpause first
    if state.auto_unpause_at > 0 && env.ledger().timestamp() >= state.auto_unpause_at {
        let mut s = state.clone();
        s.is_paused = false;
        s.auto_unpause_at = 0;
        save_pause_state(env, &s);

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "bridge_auto_unpaused"),),
            env.ledger().timestamp(),
        );
        return Ok(());
    }

    let blocked = matches!(
        (&state.pause_type, op),
        (PauseType::FullPause, _)
            | (PauseType::DepositsOnly, BridgeOperation::InitiateTransfer)
            | (PauseType::WithdrawalsOnly, BridgeOperation::InitiateWithdrawal)
            | (PauseType::ValidationOnly, BridgeOperation::ValidatorSign)
    );

    if blocked {
        return Err(AutoTradeError::BridgePaused);
    }

    Ok(())
}

// ── Public: Auto-Unpause ─────────────────────────────────────────────────────

pub fn set_auto_unpause(
    env: &Env,
    caller: &Address,
    unpause_after_seconds: u64,
) -> Result<(), AutoTradeError> {
    if !is_admin(env, caller) {
        return Err(AutoTradeError::Unauthorized);
    }

    let mut state = get_pause_state(env);
    if !state.is_paused {
        return Err(AutoTradeError::NotPaused);
    }

    let unpause_at = env.ledger().timestamp() + unpause_after_seconds;
    state.auto_unpause_at = unpause_at;
    save_pause_state(env, &state);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "auto_unpause_scheduled"),),
        unpause_at,
    );

    Ok(())
}

// ── Public: Recovery ─────────────────────────────────────────────────────────

/// Initiate recovery — creates a checklist that must be completed before unpause.
pub fn initiate_recovery(env: &Env, caller: &Address) -> Result<u64, AutoTradeError> {
    if !is_admin(env, caller) {
        return Err(AutoTradeError::Unauthorized);
    }

    let state = get_pause_state(env);
    if !state.is_paused {
        return Err(AutoTradeError::NotPaused);
    }

    let recovery_id: u64 = env
        .storage()
        .persistent()
        .get(&EmergencyKey::RecoveryCounter)
        .unwrap_or(0u64)
        + 1;
    env.storage()
        .persistent()
        .set(&EmergencyKey::RecoveryCounter, &recovery_id);

    let sentinel = env.current_contract_address();
    let mut checks = Vec::new(env);
    for name in [
        "Verify all pending transfers resolved",
        "Confirm issue that caused pause is fixed",
        "Test bridge with small transfer",
        "Verify validator set is healthy",
        "Confirm liquidity pools functioning",
    ] {
        checks.push_back(RecoveryCheck {
            name: String::from_str(env, name),
            completed: false,
            verified_by: sentinel.clone(),
        });
    }

    let checklist = RecoveryChecklist {
        recovery_id,
        initiated_by: caller.clone(),
        initiated_at: env.ledger().timestamp(),
        checks,
        all_checks_complete: false,
    };

    env.storage()
        .persistent()
        .set(&EmergencyKey::Recovery(recovery_id), &checklist);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "recovery_initiated"), caller.clone()),
        recovery_id,
    );

    Ok(recovery_id)
}

/// Mark a single recovery check as complete.
pub fn complete_recovery_check(
    env: &Env,
    recovery_id: u64,
    check_index: u32,
    verifier: &Address,
) -> Result<(), AutoTradeError> {
    if !is_admin(env, verifier) {
        return Err(AutoTradeError::Unauthorized);
    }

    let mut checklist: RecoveryChecklist = env
        .storage()
        .persistent()
        .get(&EmergencyKey::Recovery(recovery_id))
        .ok_or(AutoTradeError::RecoveryNotFound)?;

    if check_index >= checklist.checks.len() {
        return Err(AutoTradeError::InvalidAmount);
    }

    // Rebuild Vec with the updated check (Soroban Vec items are value types)
    let mut new_checks = Vec::new(env);
    for i in 0..checklist.checks.len() {
        let mut check = checklist.checks.get(i).unwrap();
        if i == check_index {
            check.completed = true;
            check.verified_by = verifier.clone();
        }
        new_checks.push_back(check);
    }
    checklist.checks = new_checks;

    let mut all_done = true;
    for i in 0..checklist.checks.len() {
        if !checklist.checks.get(i).unwrap().completed {
            all_done = false;
            break;
        }
    }
    checklist.all_checks_complete = all_done;

    env.storage()
        .persistent()
        .set(&EmergencyKey::Recovery(recovery_id), &checklist);

    if checklist.all_checks_complete {
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "recovery_checklist_complete"),),
            recovery_id,
        );
    }

    Ok(())
}

/// Unpause after all recovery checks are complete.
pub fn unpause_bridge(
    env: &Env,
    caller: &Address,
    recovery_id: u64,
) -> Result<(), AutoTradeError> {
    if !is_admin(env, caller) {
        return Err(AutoTradeError::Unauthorized);
    }

    let checklist: RecoveryChecklist = env
        .storage()
        .persistent()
        .get(&EmergencyKey::Recovery(recovery_id))
        .ok_or(AutoTradeError::RecoveryNotFound)?;

    if !checklist.all_checks_complete {
        return Err(AutoTradeError::RecoveryIncomplete);
    }

    let mut state = get_pause_state(env);
    let paused_at = state.paused_at;
    state.is_paused = false;
    state.auto_unpause_at = 0;
    save_pause_state(env, &state);

    let duration = env.ledger().timestamp().saturating_sub(paused_at);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "bridge_unpaused"), caller.clone()),
        (recovery_id, duration),
    );

    Ok(())
}

// ── Public: Queries ──────────────────────────────────────────────────────────

pub fn get_pause_status(env: &Env) -> PauseState {
    get_pause_state(env)
}

pub fn get_recovery_checklist(env: &Env, recovery_id: u64) -> Option<RecoveryChecklist> {
    env.storage()
        .persistent()
        .get(&EmergencyKey::Recovery(recovery_id))
}
