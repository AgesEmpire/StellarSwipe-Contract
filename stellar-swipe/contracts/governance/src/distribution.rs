use soroban_sdk::{contracttype, Address, Env};

use crate::errors::GovernanceError;
use crate::{
    add_balance, checked_add, checked_div, checked_mul, checked_sub, get_distribution_state,
    get_holders, get_total_supply, get_vesting_schedules, put_distribution_state,
    put_vesting_schedules,
};

pub const BPS_DENOMINATOR: i128 = 10_000;
pub const TEAM_BPS: i128 = 2_000;
pub const EARLY_INVESTOR_BPS: i128 = 1_500;
pub const COMMUNITY_REWARDS_BPS: i128 = 3_000;
pub const LIQUIDITY_MINING_BPS: i128 = 2_000;
pub const TREASURY_BPS: i128 = 1_000;
pub const PUBLIC_SALE_BPS: i128 = 500;

pub const YEAR_SECONDS: u64 = 365 * 24 * 60 * 60;
pub const TEAM_VESTING_DURATION: u64 = 4 * YEAR_SECONDS;
pub const TEAM_CLIFF_DURATION: u64 = YEAR_SECONDS;
pub const EARLY_INVESTOR_VESTING_DURATION: u64 = 2 * YEAR_SECONDS;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VestingCategory {
    Team,
    EarlyInvestors,
    Custom,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionRecipients {
    pub team: Address,
    pub early_investors: Address,
    pub community_rewards: Address,
    pub treasury: Address,
    pub public_sale: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionAllocation {
    pub team: i128,
    pub early_investors: i128,
    pub community_rewards: i128,
    pub liquidity_mining: i128,
    pub treasury: i128,
    pub public_sale: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionState {
    pub allocation: DistributionAllocation,
    pub liquidity_reserved: i128,
    pub liquidity_claimed: i128,
    pub liquidity_reward_bps: u32,
    pub min_claim_threshold: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingSchedule {
    pub beneficiary: Address,
    pub category: VestingCategory,
    pub total_amount: i128,
    pub released_amount: i128,
    pub start_time: u64,
    pub cliff_seconds: u64,
    pub duration_seconds: u64,
}

pub fn create_allocation(total_supply: i128) -> Result<DistributionAllocation, GovernanceError> {
    let team = checked_div(checked_mul(total_supply, TEAM_BPS)?, BPS_DENOMINATOR)?;
    let early_investors = checked_div(
        checked_mul(total_supply, EARLY_INVESTOR_BPS)?,
        BPS_DENOMINATOR,
    )?;
    let community_rewards = checked_div(
        checked_mul(total_supply, COMMUNITY_REWARDS_BPS)?,
        BPS_DENOMINATOR,
    )?;
    let liquidity_mining = checked_div(
        checked_mul(total_supply, LIQUIDITY_MINING_BPS)?,
        BPS_DENOMINATOR,
    )?;
    let treasury = checked_div(checked_mul(total_supply, TREASURY_BPS)?, BPS_DENOMINATOR)?;
    let public_sale = checked_div(checked_mul(total_supply, PUBLIC_SALE_BPS)?, BPS_DENOMINATOR)?;

    let total = checked_add(
        checked_add(team, early_investors)?,
        checked_add(
            checked_add(community_rewards, liquidity_mining)?,
            checked_add(treasury, public_sale)?,
        )?,
    )?;
    if total != total_supply {
        return Err(GovernanceError::InvalidSupply);
    }

    Ok(DistributionAllocation {
        team,
        early_investors,
        community_rewards,
        liquidity_mining,
        treasury,
        public_sale,
    })
}

pub fn initialize_distribution(
    env: &Env,
    recipients: &DistributionRecipients,
    total_supply: i128,
    reward_bps: u32,
    min_claim_threshold: i128,
) -> Result<DistributionState, GovernanceError> {
    if reward_bps == 0 || min_claim_threshold <= 0 {
        return Err(GovernanceError::InvalidRewardConfig);
    }

    ensure_unique_recipients(env, recipients)?;

    let allocation = create_allocation(total_supply)?;
    add_balance(
        env,
        &recipients.community_rewards,
        allocation.community_rewards,
    )?;
    add_balance(env, &recipients.treasury, allocation.treasury)?;
    add_balance(env, &recipients.public_sale, allocation.public_sale)?;

    create_vesting_schedule(
        env,
        recipients.team.clone(),
        VestingCategory::Team,
        allocation.team,
        env.ledger().timestamp(),
        TEAM_CLIFF_DURATION,
        TEAM_VESTING_DURATION,
    )?;

    create_vesting_schedule(
        env,
        recipients.early_investors.clone(),
        VestingCategory::EarlyInvestors,
        allocation.early_investors,
        env.ledger().timestamp(),
        0,
        EARLY_INVESTOR_VESTING_DURATION,
    )?;

    let state = DistributionState {
        allocation,
        liquidity_reserved: 0,
        liquidity_claimed: 0,
        liquidity_reward_bps: reward_bps,
        min_claim_threshold,
    };
    put_distribution_state(env, &state);
    Ok(state)
}

pub fn create_vesting_schedule(
    env: &Env,
    beneficiary: Address,
    category: VestingCategory,
    total_amount: i128,
    start_time: u64,
    cliff_seconds: u64,
    duration_seconds: u64,
) -> Result<(), GovernanceError> {
    if total_amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    if duration_seconds == 0 || cliff_seconds > duration_seconds {
        return Err(GovernanceError::InvalidDuration);
    }

    let mut schedules = get_vesting_schedules(env);
    if schedules.contains_key(beneficiary.clone()) {
        return Err(GovernanceError::DuplicateSchedule);
    }

    schedules.set(
        beneficiary.clone(),
        VestingSchedule {
            beneficiary,
            category,
            total_amount,
            released_amount: 0,
            start_time,
            cliff_seconds,
            duration_seconds,
        },
    );
    put_vesting_schedules(env, &schedules);
    Ok(())
}

pub fn get_schedule(env: &Env, beneficiary: &Address) -> Result<VestingSchedule, GovernanceError> {
    get_vesting_schedules(env)
        .get(beneficiary.clone())
        .ok_or(GovernanceError::VestingScheduleNotFound)
}

pub fn releasable_amount(env: &Env, beneficiary: &Address) -> Result<i128, GovernanceError> {
    let schedule = get_schedule(env, beneficiary)?;
    let now = env.ledger().timestamp();
    let cliff_time = schedule.start_time.saturating_add(schedule.cliff_seconds);

    if now < cliff_time {
        return Ok(0);
    }

    let vested = if now
        >= schedule
            .start_time
            .saturating_add(schedule.duration_seconds)
        || schedule.duration_seconds == schedule.cliff_seconds
    {
        schedule.total_amount
    } else {
        let elapsed_after_cliff = now.saturating_sub(cliff_time);
        let vesting_window = schedule.duration_seconds - schedule.cliff_seconds;
        checked_div(
            checked_mul(schedule.total_amount, elapsed_after_cliff as i128)?,
            vesting_window as i128,
        )?
    };

    checked_sub(vested, schedule.released_amount)
}

pub fn release_vested_tokens(
    env: &Env,
    beneficiary: &Address,
) -> Result<(VestingSchedule, i128), GovernanceError> {
    let mut schedules = get_vesting_schedules(env);
    let mut schedule = schedules
        .get(beneficiary.clone())
        .ok_or(GovernanceError::VestingScheduleNotFound)?;

    let cliff_time = schedule.start_time.saturating_add(schedule.cliff_seconds);
    if env.ledger().timestamp() < cliff_time {
        return Err(GovernanceError::CliffNotReached);
    }

    let releasable = releasable_amount(env, beneficiary)?;
    if releasable <= 0 {
        return Err(GovernanceError::NothingToRelease);
    }

    schedule.released_amount = checked_add(schedule.released_amount, releasable)?;
    schedules.set(beneficiary.clone(), schedule.clone());
    put_vesting_schedules(env, &schedules);
    add_balance(env, beneficiary, releasable)?;
    Ok((schedule, releasable))
}

pub fn circulating_supply(env: &Env) -> Result<i128, GovernanceError> {
    let locked = locked_vesting_supply(env)?;
    let state = get_distribution_state(env)?;
    let unclaimed_liquidity =
        checked_sub(state.allocation.liquidity_mining, state.liquidity_claimed)?;
    checked_sub(
        checked_sub(get_total_supply(env)?, locked)?,
        unclaimed_liquidity,
    )
}

pub fn reward_for_volume(env: &Env, trading_volume: i128) -> Result<i128, GovernanceError> {
    if trading_volume <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let state = get_distribution_state(env)?;
    checked_div(
        checked_mul(trading_volume, state.liquidity_reward_bps as i128)?,
        BPS_DENOMINATOR,
    )
}

pub fn reserve_liquidity_rewards(env: &Env, amount: i128) -> Result<(), GovernanceError> {
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let mut state = get_distribution_state(env)?;
    let used = checked_add(state.liquidity_claimed, state.liquidity_reserved)?;
    let next_used = checked_add(used, amount)?;
    if next_used > state.allocation.liquidity_mining {
        return Err(GovernanceError::LiquidityPoolExhausted);
    }
    state.liquidity_reserved = checked_add(state.liquidity_reserved, amount)?;
    put_distribution_state(env, &state);
    Ok(())
}

pub fn claim_reserved_liquidity_rewards(env: &Env, amount: i128) -> Result<(), GovernanceError> {
    let mut state = get_distribution_state(env)?;
    state.liquidity_reserved = checked_sub(state.liquidity_reserved, amount)?;
    state.liquidity_claimed = checked_add(state.liquidity_claimed, amount)?;
    put_distribution_state(env, &state);
    Ok(())
}

pub fn update_reward_config(
    env: &Env,
    reward_bps: u32,
    min_claim_threshold: i128,
) -> Result<DistributionState, GovernanceError> {
    if reward_bps == 0 || min_claim_threshold <= 0 {
        return Err(GovernanceError::InvalidRewardConfig);
    }
    let mut state = get_distribution_state(env)?;
    state.liquidity_reward_bps = reward_bps;
    state.min_claim_threshold = min_claim_threshold;
    put_distribution_state(env, &state);
    Ok(state)
}

pub fn distribution_state(env: &Env) -> Result<DistributionState, GovernanceError> {
    get_distribution_state(env)
}

fn locked_vesting_supply(env: &Env) -> Result<i128, GovernanceError> {
    let holders = get_holders(env);
    let schedules = get_vesting_schedules(env);
    let mut total = 0i128;
    let mut index = 0;

    while index < holders.len() {
        let holder = holders.get(index).unwrap();
        if let Some(schedule) = schedules.get(holder) {
            total = checked_add(
                total,
                checked_sub(schedule.total_amount, schedule.released_amount)?,
            )?;
        }
        index += 1;
    }

    Ok(total)
}

fn ensure_unique_recipients(
    env: &Env,
    recipients: &DistributionRecipients,
) -> Result<(), GovernanceError> {
    let addresses = soroban_sdk::vec![
        env,
        recipients.team.clone(),
        recipients.early_investors.clone(),
        recipients.community_rewards.clone(),
        recipients.treasury.clone(),
        recipients.public_sale.clone()
    ];

    let mut i = 0;
    while i < addresses.len() {
        let current = addresses.get(i).unwrap();
        let mut j = i + 1;
        while j < addresses.len() {
            if current == addresses.get(j).unwrap() {
                return Err(GovernanceError::DuplicateRecipient);
            }
            j += 1;
        }
        i += 1;
    }
    Ok(())
}
