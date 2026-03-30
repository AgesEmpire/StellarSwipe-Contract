use soroban_sdk::{contracttype, Address, Env, Map, String};
use stellar_swipe_common::emergency::{
    CircuitBreakerConfig, CircuitBreakerStats, PauseState, CAT_ALL, CAT_TRADING,
};

use crate::errors::AutoTradeError;

#[contracttype]
pub enum AdminStorageKey {
    Admin,
    Guardian,
    OracleAddress,
    OracleCircuitBreaker,
    OracleWhitelist(u32), // keyed by asset_pair
    PauseStates,
    CircuitBreakerStats,
    CircuitBreakerConfig,
    PendingAdmin,
    PendingAdminExpiry,
}

pub fn init_admin(env: &Env, admin: Address) {
    if env.storage().instance().has(&AdminStorageKey::Admin) {
        panic!("Already initialized");
    }
    env.storage()
        .instance()
        .set(&AdminStorageKey::Admin, &admin);

    let states: Map<String, PauseState> = Map::new(env);
    env.storage()
        .instance()
        .set(&AdminStorageKey::PauseStates, &states);

    let stats = CircuitBreakerStats {
        attempts_window: 0,
        failures_window: 0,
        window_start: env.ledger().timestamp(),
        volume_1h: 0,
        volume_24h_avg: 0,
        last_price: 0,
        last_price_time: 0,
    };
    env.storage()
        .instance()
        .set(&AdminStorageKey::CircuitBreakerStats, &stats);
}

pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&AdminStorageKey::Admin)
}

pub fn require_admin(env: &Env, caller: &Address) -> Result<(), AutoTradeError> {
    let admin = get_admin(env).ok_or(AutoTradeError::Unauthorized)?;
    if caller != &admin {
        return Err(AutoTradeError::Unauthorized);
    }
    Ok(())
}

pub fn set_guardian(env: &Env, caller: &Address, guardian: Address) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();
    env.storage().instance().set(&AdminStorageKey::Guardian, &guardian);
    env.events().publish((soroban_sdk::Symbol::new(env, "guardian_set"),), guardian);
    Ok(())
}

pub fn revoke_guardian(env: &Env, caller: &Address) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();
    let guardian: Address = env
        .storage()
        .instance()
        .get(&AdminStorageKey::Guardian)
        .ok_or(AutoTradeError::Unauthorized)?;
    env.storage().instance().remove(&AdminStorageKey::Guardian);
    env.events().publish((soroban_sdk::Symbol::new(env, "guardian_revoked"),), guardian);
    Ok(())
}

pub fn get_guardian(env: &Env) -> Option<Address> {
    env.storage().instance().get(&AdminStorageKey::Guardian)
}

fn is_guardian(env: &Env, caller: &Address) -> bool {
    get_guardian(env).map(|g| &g == caller).unwrap_or(false)
}

pub fn pause_category(
    env: &Env,
    caller: &Address,
    category: String,
    duration: Option<u64>,
    reason: String,
) -> Result<(), AutoTradeError> {
    if is_guardian(env, caller) {
        caller.require_auth();
    } else {
        require_admin(env, caller)?;
        caller.require_auth();
    }

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
    env.storage()
        .instance()
        .set(&AdminStorageKey::PauseStates, &states);

    Ok(())
}

pub fn unpause_category(
    env: &Env,
    caller: &Address,
    category: String,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();

    let mut states = get_pause_states(env);
    if states.contains_key(category.clone()) {
        states.remove(category.clone());
        env.storage()
            .instance()
            .set(&AdminStorageKey::PauseStates, &states);
    }
    Ok(())
}

pub fn get_pause_states(env: &Env) -> Map<String, PauseState> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::PauseStates)
        .unwrap_or(Map::new(env))
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
    if !state.paused {
        return false;
    }
    if let Some(auto) = state.auto_unpause_at {
        if env.ledger().timestamp() >= auto {
            return false;
        }
    }
    true
}

pub fn set_cb_config(
    env: &Env,
    caller: &Address,
    config: CircuitBreakerConfig,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();
    env.storage()
        .instance()
        .set(&AdminStorageKey::CircuitBreakerConfig, &config);
    Ok(())
}

pub fn update_cb_stats(env: &Env, failed: bool, volume: i128, price: i128) {
    let mut stats: CircuitBreakerStats = env
        .storage()
        .instance()
        .get(&AdminStorageKey::CircuitBreakerStats)
        .unwrap_or(CircuitBreakerStats {
            attempts_window: 0,
            failures_window: 0,
            window_start: env.ledger().timestamp(),
            volume_1h: 0,
            volume_24h_avg: 0,
            last_price: 0,
            last_price_time: 0,
        });
    let now = env.ledger().timestamp();

    if now >= stats.window_start + 600 {
        stats.attempts_window = 0;
        stats.failures_window = 0;
        stats.window_start = now;
    }

    stats.attempts_window += 1;
    if failed {
        stats.failures_window += 1;
    }
    stats.volume_1h += volume;
    if price > 0 {
        stats.last_price = price;
        stats.last_price_time = now;
    }

    env.storage()
        .instance()
        .set(&AdminStorageKey::CircuitBreakerStats, &stats);

    if let Some(config) = env
        .storage()
        .instance()
        .get::<_, CircuitBreakerConfig>(&AdminStorageKey::CircuitBreakerConfig)
    {
        if let Some(reason) =
            stellar_swipe_common::emergency::check_thresholds(env, &stats, &config, price)
        {
            let pause_state = PauseState {
                paused: true,
                paused_at: now,
                auto_unpause_at: None,
                reason: reason.clone(),
            };
            let mut states = get_pause_states(env);
            states.set(String::from_str(env, CAT_ALL), pause_state);
            env.storage()
                .instance()
                .set(&AdminStorageKey::PauseStates, &states);

            // In a real implementation, we would emit an event here too
        }
    }
}

// ==================== Two-Step Admin Transfer ====================
// 48 hours in seconds (using ledger seconds)
const PENDING_ADMIN_EXPIRY_LEDGERS: u64 = 48 * 60 * 60;

/// Propose a new admin (requires current admin)
pub fn propose_admin_transfer(
    env: &Env,
    caller: &Address,
    new_admin: Address,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();

    let now = env.ledger().timestamp();
    let expires_at = now + PENDING_ADMIN_EXPIRY_LEDGERS;

    // Store pending admin and expiry time
    env.storage()
        .instance()
        .set(&AdminStorageKey::PendingAdmin, &new_admin);
    env.storage()
        .instance()
        .set(&AdminStorageKey::PendingAdminExpiry, &expires_at);

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
pub fn accept_admin_transfer(env: &Env, caller: &Address) -> Result<(), AutoTradeError> {
    caller.require_auth();

    // Get current pending admin
    let pending_admin: Address = env
        .storage()
        .instance()
        .get(&AdminStorageKey::PendingAdmin)
        .ok_or(AutoTradeError::PendingAdminNotFound)?;

    // Verify caller is the pending admin
    if caller != &pending_admin {
        return Err(AutoTradeError::Unauthorized);
    }

    // Check if transfer has expired
    let expires_at: u64 = env
        .storage()
        .instance()
        .get(&AdminStorageKey::PendingAdminExpiry)
        .ok_or(AutoTradeError::PendingAdminNotFound)?;

    let now = env.ledger().timestamp();
    if now >= expires_at {
        // Clean up expired transfer
        env.storage().instance().remove(&AdminStorageKey::PendingAdmin);
        env.storage().instance().remove(&AdminStorageKey::PendingAdminExpiry);
        return Err(AutoTradeError::PendingAdminExpired);
    }

    // Get old admin for event
    let old_admin = get_admin(env).ok_or(AutoTradeError::Unauthorized)?;

    // Complete the transfer
    env.storage()
        .instance()
        .set(&AdminStorageKey::Admin, &pending_admin);

    // Clean up pending admin entries
    env.storage().instance().remove(&AdminStorageKey::PendingAdmin);
    env.storage().instance().remove(&AdminStorageKey::PendingAdminExpiry);

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
pub fn cancel_admin_transfer(env: &Env, caller: &Address) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();

    // Check if there's a pending transfer
    let _pending_admin: Address = env
        .storage()
        .instance()
        .get(&AdminStorageKey::PendingAdmin)
        .ok_or(AutoTradeError::PendingAdminNotFound)?;

    // Remove pending transfer
    env.storage().instance().remove(&AdminStorageKey::PendingAdmin);
    env.storage().instance().remove(&AdminStorageKey::PendingAdminExpiry);

    Ok(())
}
