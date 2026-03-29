#![allow(dead_code)]
use soroban_sdk::{contracttype, symbol_short, Address, Env};

use crate::auth::{AuthConfig, AuthKey};

#[contracttype]
#[derive(Clone)]
pub struct Signal {
    pub signal_id: u64,
    pub price: i128,
    pub expiry: u64,
    pub base_asset: u32,
}

#[contracttype]
pub enum DataKey {
    Trades(Address, u64),
    Signal(u64),
}

/// Get a signal by ID
pub fn get_signal(env: &Env, id: u64) -> Option<Signal> {
    env.storage().persistent().get(&DataKey::Signal(id))
}

/// Set a signal
pub fn set_signal(env: &Env, id: u64, signal: &Signal) {
    env.storage().persistent().set(&DataKey::Signal(id), signal);
/// Test helper: auth plus max temporary SDEX balance.
pub fn authorize_user(env: &Env, user: &Address) {
    authorize_user_with_limits(env, user, i128::MAX / 4, 30);
    env.storage()
        .temporary()
        .set(&(user.clone(), symbol_short!("balance")), &i128::MAX);
}

pub fn authorize_user_with_limits(
    env: &Env,
    user: &Address,
    max_trade_amount: i128,
    duration_days: u32,
) {
    let config = AuthConfig {
        authorized: true,
        max_trade_amount,
        expires_at: env.ledger().timestamp() + (duration_days as u64 * 86400),
        granted_at: env.ledger().timestamp(),
    };
    env.storage()
        .persistent()
        .set(&AuthKey::Authorization(user.clone()), &config);
    env.storage()
        .temporary()
        .set(&(user.clone(), symbol_short!("balance")), &i128::MAX);
}

pub fn revoke_user_authorization(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&AuthKey::Authorization(user.clone()));
}

#[cfg(test)]
pub fn authorize_user(env: &Env, user: &Address) {
    authorize_user_with_limits(env, user, 1_000_000_000_000, 30);
}
