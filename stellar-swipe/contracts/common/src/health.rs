//! Shared contract health reporting for monitoring and front-end probes.

use crate::constants::PLACEHOLDER_ADMIN_STR;
use soroban_sdk::{contracttype, Address, Env, String, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthStatus {
    pub is_initialized: bool,
    pub is_paused: bool,
    pub version: String,
    pub admin: Address,
    /// Ledger timestamp (seconds) when the contract was initialized,
    /// used by off-chain monitoring to compute uptime.
    pub initialized_at: u64,
}

/// Placeholder admin when the contract has no admin in storage (uninitialized or missing key).
pub fn placeholder_admin(env: &Env) -> Address {
    Address::from_str(env, PLACEHOLDER_ADMIN_STR)
}

/// Default row for uninitialized or unreadable state (never panics).
pub fn health_uninitialized(env: &Env, version: String) -> HealthStatus {
    HealthStatus {
        is_initialized: false,
        is_paused: false,
        version,
        admin: placeholder_admin(env),
        initialized_at: 0,
    }
}

/// Emit a health-check event so off-chain monitoring can subscribe
/// to contract state changes without polling.
pub fn emit_health_event(env: &Env, status: &HealthStatus) {
    env.events().publish(
        (Symbol::new(env, "health"),),
        HealthStatus {
            is_initialized: status.is_initialized,
            is_paused: status.is_paused,
            version: status.version.clone(),
            admin: status.admin.clone(),
            initialized_at: status.initialized_at,
        },
    );
}
