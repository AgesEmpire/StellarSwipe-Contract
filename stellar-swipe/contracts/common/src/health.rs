//! Shared contract health reporting for monitoring and front-end probes.

use crate::constants::PLACEHOLDER_ADMIN_STR;
use soroban_sdk::{contracttype, Address, Env, String};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthStatus {
    pub is_initialized: bool,
    pub is_paused: bool,
    pub version: String,
    pub admin: Address,
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
    }
}
