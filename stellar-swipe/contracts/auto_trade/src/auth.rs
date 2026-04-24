use soroban_sdk::{contracttype, Address, Env, Symbol};

use crate::errors::AutoTradeError;

const SECONDS_PER_DAY: u64 = 86400;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthConfig {
    pub authorized: bool,
    pub max_trade_amount: i128,
    pub expires_at: u64,
    pub granted_at: u64,
}

#[contracttype]
pub enum AuthKey {
    Authorization(Address),
}

/// Grant authorization to the contract to execute trades
pub fn grant_authorization(
    env: &Env,
    user: &Address,
    max_amount: i128,
    duration_days: u32,
) -> Result<(), AutoTradeError> {
    if !cfg!(test) {
        user.require_auth();
    }

    if max_amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    let current_time = env.ledger().timestamp();
    let expires_at = current_time + (duration_days as u64 * SECONDS_PER_DAY);

    let config = AuthConfig {
        authorized: true,
        max_trade_amount: max_amount,
        expires_at,
        granted_at: current_time,
    };

    env.storage()
        .persistent()
        .set(&AuthKey::Authorization(user.clone()), &config);

    #[allow(deprecated)]
    env.events()
        .publish((Symbol::new(env, "auth_granted"), user.clone()), config);

    Ok(())
}

/// Revoke authorization
pub fn revoke_authorization(env: &Env, user: &Address) -> Result<(), AutoTradeError> {
    if !cfg!(test) {
        user.require_auth();
    }

    env.storage()
        .persistent()
        .remove(&AuthKey::Authorization(user.clone()));

    #[allow(deprecated)]
    env.events()
        .publish((Symbol::new(env, "auth_revoked"), user.clone()), ());

    Ok(())
}

/// Check if user is authorized for a specific trade amount
pub fn is_authorized(env: &Env, user: &Address, amount: i128) -> bool {
    let config: Option<AuthConfig> = env
        .storage()
        .persistent()
        .get(&AuthKey::Authorization(user.clone()));

    match config {
        Some(cfg) => {
            let current_time = env.ledger().timestamp();
            cfg.authorized && current_time < cfg.expires_at && amount <= cfg.max_trade_amount
        }
        None => false,
    }
}

/// Get authorization config for a user
pub fn get_auth_config(env: &Env, user: &Address) -> Option<AuthConfig> {
    env.storage()
        .persistent()
        .get(&AuthKey::Authorization(user.clone()))
}
