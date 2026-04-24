use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::distribution::{
    circulating_supply, distribution_state, reward_for_volume, BPS_DENOMINATOR,
};
use crate::errors::GovernanceError;
use crate::{
    add_balance, add_staked_balance, checked_add, checked_div, checked_mul, checked_sub,
    get_balance, get_holders, get_pending_rewards, get_staked_balance, get_vote_locks,
    put_pending_rewards, put_vote_locks, require_initialized, subtract_balance,
    subtract_staked_balance, track_holder,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenMetadata {
    pub name: soroban_sdk::String,
    pub symbol: soroban_sdk::String,
    pub decimals: u32,
    pub total_supply: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HolderBalance {
    pub holder: Address,
    pub balance: i128,
    pub staked: i128,
    pub voting_power: i128,
    pub total: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HolderAnalytics {
    pub total_holders: u32,
    pub total_staked: i128,
    pub circulating_supply: i128,
    pub staking_ratio_bps: i128,
    pub concentration_gini_bps: i128,
    pub top_holders: Vec<HolderBalance>,
}

pub fn stake(env: &Env, user: &Address, amount: i128) -> Result<(), GovernanceError> {
    require_initialized(env)?;
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    subtract_balance(env, user, amount)?;
    add_staked_balance(env, user, amount)?;
    Ok(())
}

pub fn unstake(env: &Env, user: &Address, amount: i128) -> Result<(), GovernanceError> {
    require_initialized(env)?;
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    if get_vote_locks(env).get(user.clone()).unwrap_or(0u32) > 0 {
        return Err(GovernanceError::ActiveVoteLock);
    }
    subtract_staked_balance(env, user, amount)?;
    add_balance(env, user, amount)?;
    Ok(())
}

pub fn accrue_liquidity_rewards(
    env: &Env,
    beneficiary: &Address,
    trading_volume: i128,
) -> Result<i128, GovernanceError> {
    require_initialized(env)?;
    let reward = reward_for_volume(env, trading_volume)?;
    if reward <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    crate::distribution::reserve_liquidity_rewards(env, reward)?;
    let mut rewards = get_pending_rewards(env);
    let current = rewards.get(beneficiary.clone()).unwrap_or(0);
    rewards.set(beneficiary.clone(), checked_add(current, reward)?);
    put_pending_rewards(env, &rewards);
    track_holder(env, beneficiary);
    Ok(reward)
}

pub fn claim_liquidity_rewards(env: &Env, beneficiary: &Address) -> Result<i128, GovernanceError> {
    require_initialized(env)?;
    let mut rewards = get_pending_rewards(env);
    let pending = rewards.get(beneficiary.clone()).unwrap_or(0);
    let state = distribution_state(env)?;

    if pending <= 0 {
        return Err(GovernanceError::NothingToRelease);
    }
    if pending < state.min_claim_threshold {
        return Err(GovernanceError::BelowMinimumClaim);
    }

    rewards.set(beneficiary.clone(), 0);
    put_pending_rewards(env, &rewards);
    crate::distribution::claim_reserved_liquidity_rewards(env, pending)?;
    add_balance(env, beneficiary, pending)?;
    Ok(pending)
}

pub fn set_vote_lock(
    env: &Env,
    holder: &Address,
    active_votes: u32,
) -> Result<(), GovernanceError> {
    require_initialized(env)?;
    let mut vote_locks = get_vote_locks(env);
    vote_locks.set(holder.clone(), active_votes);
    put_vote_locks(env, &vote_locks);
    track_holder(env, holder);
    Ok(())
}

pub fn analytics(env: &Env, top_n: u32) -> Result<HolderAnalytics, GovernanceError> {
    require_initialized(env)?;

    let holders = get_holders(env);
    let mut positions: Vec<HolderBalance> = Vec::new(env);
    let mut total_staked = 0i128;
    let mut total_holder_supply = 0i128;
    let mut idx = 0;

    while idx < holders.len() {
        let holder = holders.get(idx).unwrap();
        let balance = get_balance(env, &holder);
        let staked = get_staked_balance(env, &holder);
        let total = checked_add(balance, staked)?;
        if total > 0 {
            total_staked = checked_add(total_staked, staked)?;
            total_holder_supply = checked_add(total_holder_supply, total)?;
            positions.push_back(HolderBalance {
                holder,
                balance,
                staked,
                voting_power: staked,
                total,
            });
        }
        idx += 1;
    }

    sort_holders_desc(&mut positions);

    let mut top_holders = Vec::new(env);
    let limit = if top_n < positions.len() {
        top_n
    } else {
        positions.len()
    };
    let mut index = 0;
    while index < limit {
        top_holders.push_back(positions.get(index).unwrap());
        index += 1;
    }

    let circulating = circulating_supply(env)?;
    let staking_ratio_bps = if circulating <= 0 {
        0
    } else {
        checked_div(checked_mul(total_staked, BPS_DENOMINATOR)?, circulating)?
    };

    Ok(HolderAnalytics {
        total_holders: positions.len(),
        total_staked,
        circulating_supply: circulating,
        staking_ratio_bps,
        concentration_gini_bps: gini_bps(env, &positions, total_holder_supply)?,
        top_holders,
    })
}

fn sort_holders_desc(holders: &mut Vec<HolderBalance>) {
    if holders.len() < 2 {
        return;
    }
    let mut i = 0;
    while i < holders.len() {
        let mut j = 0;
        while j + 1 < holders.len() - i {
            let left = holders.get(j).unwrap();
            let right = holders.get(j + 1).unwrap();
            if right.total > left.total {
                holders.set(j, right);
                holders.set(j + 1, left);
            }
            j += 1;
        }
        i += 1;
    }
}

fn gini_bps(
    env: &Env,
    holders: &Vec<HolderBalance>,
    total_holder_supply: i128,
) -> Result<i128, GovernanceError> {
    if holders.len() <= 1 || total_holder_supply <= 0 {
        return Ok(0);
    }

    let mut balances: Vec<i128> = Vec::new(env);
    let mut index = 0;
    while index < holders.len() {
        balances.push_back(holders.get(index).unwrap().total);
        index += 1;
    }
    sort_i128_asc(&mut balances);

    let n = balances.len() as i128;
    let mut weighted_sum = 0i128;
    let mut i = 0;
    while i < balances.len() {
        let ordinal = (i + 1) as i128;
        weighted_sum = checked_add(
            weighted_sum,
            checked_mul(ordinal, balances.get(i).unwrap())?,
        )?;
        i += 1;
    }

    let numerator = checked_mul(
        checked_sub(
            checked_mul(2, weighted_sum)?,
            checked_mul(n + 1, total_holder_supply)?,
        )?,
        BPS_DENOMINATOR,
    )?;
    let denominator = checked_mul(n, total_holder_supply)?;
    if denominator <= 0 {
        return Ok(0);
    }

    let value = checked_div(numerator, denominator)?;
    if value < 0 {
        Ok(0)
    } else {
        Ok(value)
    }
}

fn sort_i128_asc(values: &mut Vec<i128>) {
    if values.len() < 2 {
        return;
    }
    let mut i = 0;
    while i < values.len() {
        let mut j = 0;
        while j + 1 < values.len() - i {
            let left = values.get(j).unwrap();
            let right = values.get(j + 1).unwrap();
            if right < left {
                values.set(j, right);
                values.set(j + 1, left);
            }
            j += 1;
        }
        i += 1;
    }
}
