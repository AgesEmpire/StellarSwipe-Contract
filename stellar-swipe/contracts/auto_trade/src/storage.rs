#![allow(dead_code)]
use soroban_sdk::{contracttype, Address, Env};

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
}

#[cfg(test)]
pub fn authorize_user(env: &Env, user: &Address) {
    let config = crate::auth::AuthConfig {
        authorized: true,
        max_trade_amount: 1_000_000_000_000,
        expires_at: env.ledger().timestamp() + (30 * 86400),
        granted_at: env.ledger().timestamp(),
    };
    env.storage()
        .persistent()
        .set(&crate::auth::AuthKey::Authorization(user.clone()), &config);
}
