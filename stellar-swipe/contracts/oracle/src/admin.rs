use soroban_sdk::{Address, Env, Map, String, contracttype};
use stellar_swipe_common::emergency::{PauseState, CAT_ALL};

use crate::errors::OracleError;
use crate::types::StorageKey;

pub fn pause_category(
    env: &Env,
    caller: &Address,
    category: String,
    duration: Option<u64>,
    reason: String,
) -> Result<(), OracleError> {
    require_admin(env, caller)?;
    caller.require_auth();

    let now = env.ledger().timestamp();
    let auto_unpause_at = duration.map(|d| now + d);

    let pause_state = PauseState {
        paused: true,
        paused_at: now,
        auto_unpause_at,
        reason: reason.clone(),
    };

    let mut states = get_pause_states(env);
    states.set(category.clone(), pause_state);
    env.storage().instance().set(&StorageKey::PauseStates, &states);

    Ok(())
}

pub fn unpause_category(env: &Env, caller: &Address, category: String) -> Result<(), OracleError> {
    require_admin(env, caller)?;
    caller.require_auth();

    let mut states = get_pause_states(env);
    if states.contains_key(category.clone()) {
        states.remove(category.clone());
        env.storage().instance().set(&StorageKey::PauseStates, &states);
    }
    Ok(())
}

pub fn get_pause_states(env: &Env) -> Map<String, PauseState> {
    env.storage().instance().get(&StorageKey::PauseStates).unwrap_or(Map::new(env))
}

pub fn is_paused(env: &Env, category: String) -> bool {
    let states = get_pause_states(env);
    
    if let Some(all_pause) = states.get(String::from_str(env, CAT_ALL)) {
        if is_state_active(env, &all_pause) {
            return true;
        }
    }
    
    if let Some(pause) = states.get(category) {
        return is_state_active(env, &pause);
    }
    
    false
}

fn is_state_active(env: &Env, state: &PauseState) -> bool {
    if !state.paused { return false; }
    if let Some(auto) = state.auto_unpause_at {
        if env.ledger().timestamp() >= auto { return false; }
    }
    true
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), OracleError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(OracleError::Unauthorized)?;

    if caller != &admin {
        return Err(OracleError::Unauthorized);
    }
    Ok(())
}

// ==================== Two-Step Admin Transfer ====================
// 48 hours in seconds (using ledger seconds)
const PENDING_ADMIN_EXPIRY_LEDGERS: u64 = 48 * 60 * 60;

/// Propose a new admin (requires current admin)
pub fn propose_admin_transfer(
    env: &Env,
    caller: &Address,
    new_admin: Address,
) -> Result<(), OracleError> {
    require_admin(env, caller)?;
    caller.require_auth();

    let now = env.ledger().timestamp();
    let expires_at = now + PENDING_ADMIN_EXPIRY_LEDGERS;

    // Store pending admin and expiry time
    env.storage()
        .instance()
        .set(&StorageKey::PendingAdmin, &new_admin);
    env.storage()
        .instance()
        .set(&StorageKey::PendingAdminExpiry, &expires_at);

    // Emit event
    env.events().publish(
        (
            soroban_sdk::Symbol::new(env, "admin_transfer_proposed"),
            caller.clone(),
            new_admin,
        ),
        expires_at,
    );

    Ok(())
}

/// Accept admin transfer (called by new admin)
pub fn accept_admin_transfer(env: &Env, caller: &Address) -> Result<(), OracleError> {
    caller.require_auth();

    // Get current pending admin
    let pending_admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::PendingAdmin)
        .ok_or(OracleError::PendingAdminNotFound)?;

    // Verify caller is the pending admin
    if caller != &pending_admin {
        return Err(OracleError::Unauthorized);
    }

    // Check if transfer has expired
    let expires_at: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::PendingAdminExpiry)
        .ok_or(OracleError::PendingAdminNotFound)?;

    let now = env.ledger().timestamp();
    if now >= expires_at {
        // Clean up expired transfer
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        env.storage().instance().remove(&StorageKey::PendingAdminExpiry);
        return Err(OracleError::PendingAdminExpired);
    }

    // Get old admin for event
    let old_admin = env
        .storage()
        .instance()
        .get::<_, Address>(&StorageKey::Admin)
        .ok_or(OracleError::Unauthorized)?;

    // Complete the transfer
    env.storage()
        .instance()
        .set(&StorageKey::Admin, &pending_admin);

    // Clean up pending admin entries
    env.storage().instance().remove(&StorageKey::PendingAdmin);
    env.storage().instance().remove(&StorageKey::PendingAdminExpiry);

    // Emit completion event
    env.events().publish(
        (
            soroban_sdk::Symbol::new(env, "admin_transfer_completed"),
            old_admin,
            pending_admin,
        ),
        (),
    );

    Ok(())
}

/// Cancel pending admin transfer (current admin only)
pub fn cancel_admin_transfer(env: &Env, caller: &Address) -> Result<(), OracleError> {
    require_admin(env, caller)?;
    caller.require_auth();

    // Check if there's a pending transfer
    let _pending_admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::PendingAdmin)
        .ok_or(OracleError::PendingAdminNotFound)?;

    // Remove pending transfer
    env.storage().instance().remove(&StorageKey::PendingAdmin);
    env.storage().instance().remove(&StorageKey::PendingAdminExpiry);

    Ok(())
}
