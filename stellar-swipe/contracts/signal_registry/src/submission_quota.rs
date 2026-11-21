//! Per-provider and global ledger-window submission quotas (Issue #1138).
//!
//! Windows are `ledger_sequence / window_ledgers`, so counters reset
//! deterministically at window boundaries.  [`check`] is read-only and runs
//! before any state is written; [`consume`] runs only after a submission has
//! fully succeeded, so a rejected submission leaves no partial state.
//! A limit of `0` disables that quota.

use crate::errors::AdminError;
use soroban_sdk::{contracttype, Address, Env};

/// Bounds for administrator-configured limits.
pub const MIN_WINDOW_LEDGERS: u32 = 1;
pub const MAX_WINDOW_LEDGERS: u32 = 535_680; // ~31 days at 5s ledgers
pub const MAX_PROVIDER_LIMIT: u32 = 1_000;
pub const MAX_GLOBAL_LIMIT: u32 = 100_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuotaConfig {
    pub window_ledgers: u32,
    pub per_provider_limit: u32,
    pub global_limit: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum QuotaKey {
    Config,
    /// (provider, window) -> count
    Provider(Address, u32),
    /// window -> count
    Global(u32),
}

pub fn validate(config: &QuotaConfig) -> Result<(), AdminError> {
    if config.window_ledgers < MIN_WINDOW_LEDGERS
        || config.window_ledgers > MAX_WINDOW_LEDGERS
        || config.per_provider_limit > MAX_PROVIDER_LIMIT
        || config.global_limit > MAX_GLOBAL_LIMIT
        || (config.global_limit != 0 && config.per_provider_limit > config.global_limit)
    {
        return Err(AdminError::InvalidParameter);
    }
    Ok(())
}

pub fn set_config(env: &Env, config: &QuotaConfig) -> Result<(), AdminError> {
    validate(config)?;
    env.storage().instance().set(&QuotaKey::Config, config);
    Ok(())
}

pub fn get_config(env: &Env) -> Option<QuotaConfig> {
    env.storage().instance().get(&QuotaKey::Config)
}

pub fn current_window(env: &Env, config: &QuotaConfig) -> u32 {
    env.ledger().sequence() / config.window_ledgers
}

fn count(env: &Env, key: &QuotaKey) -> u32 {
    env.storage().temporary().get(key).unwrap_or(0)
}

/// Returns `(provider_used, global_used)` for the current window.
pub fn usage(env: &Env, provider: &Address) -> (u32, u32) {
    match get_config(env) {
        None => (0, 0),
        Some(c) => {
            let w = current_window(env, &c);
            (
                count(env, &QuotaKey::Provider(provider.clone(), w)),
                count(env, &QuotaKey::Global(w)),
            )
        }
    }
}

/// Read-only quota check; performs no writes.
pub fn check(env: &Env, provider: &Address) -> Result<(), AdminError> {
    let Some(c) = get_config(env) else { return Ok(()) };
    let w = current_window(env, &c);
    if c.per_provider_limit > 0
        && count(env, &QuotaKey::Provider(provider.clone(), w)) >= c.per_provider_limit
    {
        return Err(AdminError::RateLimitExceeded);
    }
    if c.global_limit > 0 && count(env, &QuotaKey::Global(w)) >= c.global_limit {
        return Err(AdminError::RateLimitExceeded);
    }
    Ok(())
}

/// Records one accepted submission.  Call only after the submission succeeded.
pub fn consume(env: &Env, provider: &Address) {
    let Some(c) = get_config(env) else { return };
    let w = current_window(env, &c);
    // Counters live one window past their own so they expire on their own.
    let ttl = c.window_ledgers.saturating_mul(2);
    for key in [QuotaKey::Provider(provider.clone(), w), QuotaKey::Global(w)] {
        let next = count(env, &key).saturating_add(1);
        env.storage().temporary().set(&key, &next);
        env.storage().temporary().extend_ttl(&key, ttl, ttl);
    }
}
