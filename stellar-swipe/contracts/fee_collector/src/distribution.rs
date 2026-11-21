//! Epoch-based fee distribution to signal providers.
//!
//! Fees recorded during an epoch are split on `close_epoch`: each provider is
//! credited `floor(total * share_bps / 10_000)`; the remainder (including
//! rounding dust) is retained by the treasury, so no stroop is lost.

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::errors::ContractError;
use crate::storage::{get_treasury_balance, set_treasury_balance};

pub const BPS_DENOMINATOR: u32 = 10_000;

#[contracttype]
#[derive(Clone)]
pub struct ProviderShare {
    pub provider: Address,
    pub share_bps: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DistKey {
    Providers,
    EpochFees(Address),             // token -> fees collected in current epoch
    ProviderBalance(Address, Address), // (provider, token)
    Epoch,
}

pub fn set_providers(env: &Env, providers: &Vec<ProviderShare>) -> Result<(), ContractError> {
    let mut total: u32 = 0;
    for p in providers.iter() {
        total = total
            .checked_add(p.share_bps)
            .ok_or(ContractError::ArithmeticOverflow)?;
    }
    if total > BPS_DENOMINATOR {
        return Err(ContractError::InvalidAmount);
    }
    env.storage().instance().set(&DistKey::Providers, providers);
    Ok(())
}

pub fn get_providers(env: &Env) -> Vec<ProviderShare> {
    env.storage()
        .instance()
        .get(&DistKey::Providers)
        .unwrap_or(Vec::new(env))
}

pub fn epoch_fees(env: &Env, token: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DistKey::EpochFees(token.clone()))
        .unwrap_or(0)
}

pub fn record_fee(env: &Env, token: &Address, amount: i128) -> Result<(), ContractError> {
    if amount <= 0 {
        return Err(ContractError::InvalidAmount);
    }
    let total = epoch_fees(env, token)
        .checked_add(amount)
        .ok_or(ContractError::ArithmeticOverflow)?;
    env.storage()
        .persistent()
        .set(&DistKey::EpochFees(token.clone()), &total);
    Ok(())
}

pub fn provider_balance(env: &Env, provider: &Address, token: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DistKey::ProviderBalance(provider.clone(), token.clone()))
        .unwrap_or(0)
}

pub fn current_epoch(env: &Env) -> u32 {
    env.storage().instance().get(&DistKey::Epoch).unwrap_or(0)
}

/// Distributes the epoch's fees and returns the amount retained by treasury.
pub fn close_epoch(env: &Env, token: &Address) -> Result<i128, ContractError> {
    let total = epoch_fees(env, token);
    let mut distributed: i128 = 0;
    for p in get_providers(env).iter() {
        let share = total
            .checked_mul(p.share_bps as i128)
            .ok_or(ContractError::ArithmeticOverflow)?
            / BPS_DENOMINATOR as i128;
        let key = DistKey::ProviderBalance(p.provider.clone(), token.clone());
        let bal = provider_balance(env, &p.provider, token)
            .checked_add(share)
            .ok_or(ContractError::ArithmeticOverflow)?;
        env.storage().persistent().set(&key, &bal);
        distributed += share;
    }
    let retained = total - distributed;
    let treasury = get_treasury_balance(env, token)
        .checked_add(retained)
        .ok_or(ContractError::ArithmeticOverflow)?;
    set_treasury_balance(env, token, treasury);
    env.storage()
        .persistent()
        .remove(&DistKey::EpochFees(token.clone()));
    env.storage()
        .instance()
        .set(&DistKey::Epoch, &(current_epoch(env) + 1));
    Ok(retained)
}
