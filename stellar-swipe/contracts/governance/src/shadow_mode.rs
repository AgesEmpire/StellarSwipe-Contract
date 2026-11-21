use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, String, Vec};

use crate::{require_admin, GovernanceError, StorageKey};

/// Structured event emitted at the end of every shadow-mode simulation
/// (`simulate_proposal`). Off-chain indexers and dashboards can subscribe to
/// this event to observe simulation outcomes without an additional contract
/// call.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ShadowModeResult {
    /// The proposal being simulated.
    pub proposal_id: u64,
    /// `true` when the proposal would execute successfully.
    pub success: bool,
    /// Human-readable summaries of the state changes that would occur.
    pub simulated_state_changes: Vec<String>,
    /// `None` for successful simulations; `Some(reason)` when the simulation
    /// detects an execution failure.
    pub failure_reason: Option<String>,
}

/// State persisted during a shadow-mode upgrade trial period.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowModeState {
    /// Timestamp at which the shadow period ends.
    pub trial_ends_at: u64,
    /// WASM hash of the new logic being evaluated.
    pub new_wasm_hash: Bytes,
    /// Whether the new logic has been promoted to handle all paths.
    pub promoted: bool,
}

fn get_shadow_state(env: &Env) -> Option<ShadowModeState> {
    env.storage().instance().get(&StorageKey::ShadowMode)
}

fn put_shadow_state(env: &Env, state: &ShadowModeState) {
    env.storage().instance().set(&StorageKey::ShadowMode, state);
}

fn clear_shadow_state(env: &Env) {
    env.storage().instance().remove(&StorageKey::ShadowMode);
}

/// Admin-only: begin a shadow-mode trial for a new WASM hash.
///
/// During the trial period, designated read-only entrypoints should invoke
/// `compare_shadow_results` to emit a discrepancy event when old and new logic
/// disagree. Mutating entrypoints are unaffected and continue using the
/// already-promoted logic.
pub fn enter_shadow_mode(
    env: &Env,
    admin: &Address,
    new_wasm_hash: Bytes,
    trial_duration_seconds: u64,
) -> Result<ShadowModeState, GovernanceError> {
    require_admin(env, admin)?;
    if new_wasm_hash.len() != 32 {
        return Err(GovernanceError::InvalidProposal);
    }
    if trial_duration_seconds == 0 {
        return Err(GovernanceError::InvalidDuration);
    }
    let state = ShadowModeState {
        trial_ends_at: env
            .ledger()
            .timestamp()
            .saturating_add(trial_duration_seconds),
        new_wasm_hash,
        promoted: false,
    };
    put_shadow_state(env, &state);
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("shadow"), symbol_short!("enter")),
        (admin.clone(), state.trial_ends_at),
    );
    Ok(state)
}

/// Compare two hashed outputs (e.g. a deterministic digest of a query result).
///
/// Both `old_output_hash` and `new_output_hash` should be 32-byte digests of
/// the outputs produced by old and new logic respectively for the same input.
///
/// When the hashes differ a `shadow/discrepancy` event is emitted for
/// monitoring. The discrepancy does **not** affect the value returned to the
/// caller — the caller should always use the result from the promoted logic.
///
/// Returns `true` if the outputs match, `false` if a discrepancy was detected.
pub fn compare_shadow_results(
    env: &Env,
    entrypoint_id: u32,
    old_output_hash: Bytes,
    new_output_hash: Bytes,
) -> bool {
    let state = match get_shadow_state(env) {
        Some(s) => s,
        None => return true,
    };
    // Outside the trial window — shadow comparison is a no-op.
    if state.promoted || env.ledger().timestamp() > state.trial_ends_at {
        return true;
    }
    let matched = old_output_hash == new_output_hash;
    if !matched {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("shadow"), symbol_short!("discrep")),
            (entrypoint_id, old_output_hash, new_output_hash),
        );
    }
    matched
}

/// Return whether the contract is currently in shadow mode (trial active).
pub fn is_in_shadow_mode(env: &Env) -> bool {
    match get_shadow_state(env) {
        Some(s) => !s.promoted && env.ledger().timestamp() <= s.trial_ends_at,
        None => false,
    }
}

/// Admin-only: end the shadow trial and promote the new logic for all paths.
pub fn promote_from_shadow_mode(env: &Env, admin: &Address) -> Result<(), GovernanceError> {
    require_admin(env, admin)?;
    let mut state = get_shadow_state(env).ok_or(GovernanceError::NotInitialized)?;
    state.promoted = true;
    put_shadow_state(env, &state);
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("shadow"), symbol_short!("promote")),
        admin.clone(),
    );
    Ok(())
}

/// Admin-only: cancel the shadow trial without promoting.
pub fn cancel_shadow_mode(env: &Env, admin: &Address) -> Result<(), GovernanceError> {
    require_admin(env, admin)?;
    if get_shadow_state(env).is_none() {
        return Err(GovernanceError::NotInitialized);
    }
    clear_shadow_state(env);
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("shadow"), symbol_short!("cancel")),
        admin.clone(),
    );
    Ok(())
}
