//! Strategy-level slippage settings validated against protocol bounds (issue #1367).

use soroban_sdk::{contracttype, Address, Env};

use crate::errors::AutoTradeError;

/// Lowest accepted slippage setting (0.01%).
pub const MIN_SLIPPAGE_BPS: u32 = 1;
/// Protocol safety ceiling (10%).
pub const PROTOCOL_MAX_SLIPPAGE_BPS: u32 = 1_000;

#[contracttype]
enum SlippageKey {
    Strategy(Address, u64),
}

pub fn validate(bps: u32) -> Result<u32, AutoTradeError> {
    if bps < MIN_SLIPPAGE_BPS || bps > PROTOCOL_MAX_SLIPPAGE_BPS {
        return Err(AutoTradeError::InvalidSlippageSetting);
    }
    Ok(bps)
}

pub fn set(env: &Env, user: &Address, strategy_id: u64, bps: u32) -> Result<(), AutoTradeError> {
    validate(bps)?;
    env.storage()
        .persistent()
        .set(&SlippageKey::Strategy(user.clone(), strategy_id), &bps);
    Ok(())
}

pub fn get(env: &Env, user: &Address, strategy_id: u64) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&SlippageKey::Strategy(user.clone(), strategy_id))
}

/// Effective bound used right before execution: the tighter of the request and
/// the stored strategy setting, rechecked against the protocol bounds.
pub fn effective(
    env: &Env,
    user: &Address,
    strategy_id: u64,
    requested_bps: u32,
) -> Result<u32, AutoTradeError> {
    let bound = match get(env, user, strategy_id) {
        Some(stored) => core::cmp::min(stored, requested_bps),
        None => requested_bps,
    };
    validate(bound)
}
