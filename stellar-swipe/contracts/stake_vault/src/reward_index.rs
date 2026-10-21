//! Reward index checkpointing (Issue #1132).
//!
//! Classic accumulator: each reward asset keeps a global `index` (reward per
//! unit of participating stake, scaled by [`INDEX_SCALE`]). Every stake change
//! (deposit, withdrawal, partial unstake) and every claim first checkpoints
//! the staker: rewards accrued since their last checkpoint are credited using
//! their *old* participating stake, then their snapshot index is reset to the
//! current global index. A late deposit therefore starts at the current index
//! and cannot claim rewards funded before it participated.
//!
//! ## Rounding
//! * Index increments round **down**; the remainder (in scaled units) is
//!   carried in `dust` and added to the next funding, so nothing is lost.
//! * Per-staker accrual rounds **down**. The sum of credited rewards is
//!   therefore always <= funded rewards (conservation); sub-unit residue stays
//!   in the vault.
//!
//! ## Full exit
//! When a staker's participating stake reaches zero their snapshot index and
//! stake entries are removed, so no stale index remains. Any already-accrued
//! balance stays claimable.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

use crate::StakeVaultError;

pub const INDEX_SCALE: i128 = 1_000_000_000_000;
/// Bound on indexed reward assets so checkpoints stay O(small).
pub const MAX_INDEX_ASSETS: u32 = 8;

#[contracttype]
#[derive(Clone)]
pub enum RewardIndexKey {
    Assets,
    TotalStake,
    Index(Address),
    Dust(Address),
    UserStake(Address),
    UserIndex(Address, Address),
    Accrued(Address, Address),
}

fn get_i128(env: &Env, k: &RewardIndexKey) -> i128 {
    env.storage().persistent().get(k).unwrap_or(0)
}

fn put_i128(env: &Env, k: &RewardIndexKey, v: i128) {
    if v == 0 {
        env.storage().persistent().remove(k);
    } else {
        env.storage().persistent().set(k, &v);
    }
}

pub fn assets(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&RewardIndexKey::Assets)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn total_stake(env: &Env) -> i128 {
    get_i128(env, &RewardIndexKey::TotalStake)
}

pub fn user_stake(env: &Env, staker: &Address) -> i128 {
    get_i128(env, &RewardIndexKey::UserStake(staker.clone()))
}

pub fn global_index(env: &Env, asset: &Address) -> i128 {
    get_i128(env, &RewardIndexKey::Index(asset.clone()))
}

pub fn user_index(env: &Env, staker: &Address, asset: &Address) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&RewardIndexKey::UserIndex(staker.clone(), asset.clone()))
}

pub fn add_asset(env: &Env, asset: Address) -> Result<(), StakeVaultError> {
    let mut list = assets(env);
    if list.contains(&asset) {
        return Ok(());
    }
    if list.len() >= MAX_INDEX_ASSETS {
        return Err(StakeVaultError::QueueFull);
    }
    list.push_back(asset);
    env.storage().persistent().set(&RewardIndexKey::Assets, &list);
    Ok(())
}

/// Credit `amount` of `asset` to all currently participating stake.
/// Caller is responsible for moving the tokens into the vault.
pub fn notify_reward(env: &Env, asset: &Address, amount: i128) -> Result<i128, StakeVaultError> {
    if amount <= 0 {
        return Err(StakeVaultError::InvalidAmount);
    }
    if !assets(env).contains(asset) {
        return Err(StakeVaultError::InvalidAmount);
    }
    let total = total_stake(env);
    if total <= 0 {
        // No participants: refuse rather than strand the funds.
        return Err(StakeVaultError::NoStake);
    }
    let dust_key = RewardIndexKey::Dust(asset.clone());
    let num = amount
        .checked_mul(INDEX_SCALE)
        .and_then(|v| v.checked_add(get_i128(env, &dust_key)))
        .ok_or(StakeVaultError::StakeOverflow)?;
    let inc = num / total;
    put_i128(env, &dust_key, num % total);
    let idx_key = RewardIndexKey::Index(asset.clone());
    let new_index = get_i128(env, &idx_key)
        .checked_add(inc)
        .ok_or(StakeVaultError::StakeOverflow)?;
    put_i128(env, &idx_key, new_index);
    env.events().publish(
        (symbol_short!("rwidx"), symbol_short!("notify")),
        (asset.clone(), amount, total, new_index),
    );
    Ok(new_index)
}

/// Credit rewards accrued since the last checkpoint and move every asset's
/// snapshot to the current index. Uses the *recorded* participating stake.
pub fn checkpoint(env: &Env, staker: &Address) -> Result<(), StakeVaultError> {
    let stake = user_stake(env, staker);
    let list = assets(env);
    for i in 0..list.len() {
        let asset = list.get_unchecked(i);
        let g = global_index(env, &asset);
        let u = user_index(env, staker, &asset).unwrap_or(g);
        let delta = g.checked_sub(u).ok_or(StakeVaultError::StakeOverflow)?;
        if stake > 0 && delta > 0 {
            let earned = stake
                .checked_mul(delta)
                .ok_or(StakeVaultError::StakeOverflow)?
                / INDEX_SCALE;
            let acc_key = RewardIndexKey::Accrued(staker.clone(), asset.clone());
            let acc = get_i128(env, &acc_key)
                .checked_add(earned)
                .ok_or(StakeVaultError::StakeOverflow)?;
            put_i128(env, &acc_key, acc);
        }
        let ui_key = RewardIndexKey::UserIndex(staker.clone(), asset.clone());
        if stake > 0 {
            env.storage().persistent().set(&ui_key, &g);
        } else {
            env.storage().persistent().remove(&ui_key);
        }
    }
    Ok(())
}

/// Checkpoint, then record `new_stake` as the staker's participating stake.
/// Called after every stake mutation.
pub fn on_stake_change(env: &Env, staker: &Address, new_stake: i128) -> Result<(), StakeVaultError> {
    checkpoint(env, staker)?;
    let old = user_stake(env, staker);
    let total = total_stake(env)
        .checked_sub(old)
        .and_then(|t| t.checked_add(new_stake))
        .filter(|t| *t >= 0)
        .ok_or(StakeVaultError::StakeOverflow)?;
    put_i128(env, &RewardIndexKey::TotalStake, total);
    put_i128(env, &RewardIndexKey::UserStake(staker.clone()), new_stake);
    let list = assets(env);
    for i in 0..list.len() {
        let asset = list.get_unchecked(i);
        let ui_key = RewardIndexKey::UserIndex(staker.clone(), asset.clone());
        if new_stake > 0 {
            env.storage()
                .persistent()
                .set(&ui_key, &global_index(env, &asset));
        } else {
            // Full exit: drop the snapshot so no stale index remains.
            env.storage().persistent().remove(&ui_key);
        }
    }
    Ok(())
}

pub fn accrued(env: &Env, staker: &Address, asset: &Address) -> i128 {
    get_i128(env, &RewardIndexKey::Accrued(staker.clone(), asset.clone()))
}

/// Checkpoint and zero the staker's accrued balance, returning the amount.
pub fn take_accrued(env: &Env, staker: &Address, asset: &Address) -> Result<i128, StakeVaultError> {
    checkpoint(env, staker)?;
    let key = RewardIndexKey::Accrued(staker.clone(), asset.clone());
    let amt = get_i128(env, &key);
    put_i128(env, &key, 0);
    Ok(amt)
}

/// Re-credit an amount (e.g. a cancelled queued claim).
pub fn credit_accrued(
    env: &Env,
    staker: &Address,
    asset: &Address,
    amount: i128,
) -> Result<(), StakeVaultError> {
    let key = RewardIndexKey::Accrued(staker.clone(), asset.clone());
    let v = get_i128(env, &key)
        .checked_add(amount)
        .ok_or(StakeVaultError::StakeOverflow)?;
    put_i128(env, &key, v);
    Ok(())
}
