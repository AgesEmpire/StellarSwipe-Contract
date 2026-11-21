//! Stake / reward asset decimal compatibility (Issue #1136).
//!
//! Every reward asset must report the same `decimals()` as the stake token so
//! reward math never silently mixes unit scales.  Checks run *before* any
//! state mutation, on configuration (`add_reward_asset`) and on migration
//! (`migrate_stake_token`), which checks both the old and the new metadata.

use soroban_sdk::{Address, Env, Symbol, Val, Vec};

/// Largest decimals value accepted as well-formed metadata.
pub const MAX_DECIMALS: u32 = 18;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecimalsError {
    /// The asset does not expose `decimals()` (or the call failed).
    Missing,
    /// `decimals()` returned a non-u32 or an out-of-range value.
    Malformed,
    /// Both assets are well-formed but their decimals differ.
    Incompatible,
}

/// Reads and validates the `decimals()` metadata of `asset`.
pub fn read_decimals(env: &Env, asset: &Address) -> Result<u32, DecimalsError> {
    let raw = env
        .try_invoke_contract::<Val, soroban_sdk::Error>(
            asset,
            &Symbol::new(env, "decimals"),
            Vec::<Val>::new(env),
        )
        .map_err(|_| DecimalsError::Missing)?
        .map_err(|_| DecimalsError::Malformed)?;
    let decimals: u32 = soroban_sdk::TryFromVal::try_from_val(env, &raw)
        .map_err(|_| DecimalsError::Malformed)?;
    if decimals > MAX_DECIMALS {
        return Err(DecimalsError::Malformed);
    }
    Ok(decimals)
}

/// Deterministic compatibility rule: both well-formed and equal.
pub fn require_compatible(env: &Env, stake: &Address, reward: &Address) -> Result<u32, DecimalsError> {
    let s = read_decimals(env, stake)?;
    let r = read_decimals(env, reward)?;
    if s != r {
        return Err(DecimalsError::Incompatible);
    }
    Ok(s)
}

/// Migration check: old and new stake tokens must agree, and every registered
/// reward asset must stay compatible with the new token.
pub fn require_migration_compatible(
    env: &Env,
    old_stake: &Address,
    new_stake: &Address,
    reward_assets: &Vec<Address>,
) -> Result<u32, DecimalsError> {
    let decimals = require_compatible(env, old_stake, new_stake)?;
    for asset in reward_assets.iter() {
        require_compatible(env, new_stake, &asset)?;
    }
    Ok(decimals)
}

