pub const XLM: i128 = 10_000_000;
pub const LIQUIDITY_MINING_REWARD: i128 = 10 * XLM;
pub const LIQUIDITY_MINING_USER_CAP: i128 = 1_000 * XLM;
pub const DEFAULT_MINING_PERIOD_SECONDS: u64 = 90 * 24 * 60 * 60;
pub const DEFAULT_VESTING_CLIFF_SECONDS: u64 = 30 * 24 * 60 * 60;
pub const DEFAULT_VESTING_DURATION_SECONDS: u64 = 180 * 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct LiquidityMiningConfig {
    pub liquidity_mining_active: bool,
    pub mainnet_launch_timestamp: u64,
    pub mining_period_seconds: u64,
    pub treasury_balance: i128,
    /// Protected reserves that must remain untouched by withdrawals/payouts.
    pub protected_reserves: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiquidityMiningRewardEarned<User> {
    pub user: User,
    pub amount: i128,
    pub trades_remaining: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VestingSchedule {
    pub start_timestamp: u64,
    pub cliff_seconds: u64,
    pub duration_seconds: u64,
    pub total_amount: i128,
    pub released_amount: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RewardError {
    MiningInactive,
    MiningPeriodEnded,
    UserCapReached,
    InsufficientTreasury,
    CliffNotReached,
    NothingToRelease,
    /// Withdrawal would reduce the treasury below its protected solvency floor.
    SolvencyFloorBreached,
}

/// Returns the maximum amount that may be withdrawn from the treasury while
/// keeping the protected reserves at or above their configured floor.
pub fn withdrawable_amount(config: &LiquidityMiningConfig) -> i128 {
    config
        .treasury_balance
        .saturating_sub(config.protected_reserves)
        .max(0)
}

/// Enforces the treasury solvency invariant for a prospective withdrawal.
///
/// The check is pure and does not mutate any state, so callers can validate
/// atomically before committing a withdrawal. Returns
/// `RewardError::SolvencyFloorBreached` when the withdrawal would push the
/// treasury below its protected reserves.
pub fn check_solvency_invariant(
    config: &LiquidityMiningConfig,
    amount: i128,
) -> Result<(), RewardError> {
    if amount < 0 || amount > withdrawable_amount(config) {
        return Err(RewardError::SolvencyFloorBreached);
    }
    Ok(())
}

pub fn distribute_liquidity_mining_reward<User: Clone>(
    config: &mut LiquidityMiningConfig,
    user: User,
    user_rewards_earned: &mut i128,
    now: u64,
) -> Result<LiquidityMiningRewardEarned<User>, RewardError> {
    if !config.liquidity_mining_active {
        return Err(RewardError::MiningInactive);
    }

    let mining_ends_at = config
        .mainnet_launch_timestamp
        .saturating_add(config.mining_period_seconds);
    if now >= mining_ends_at {
        config.liquidity_mining_active = false;
        return Err(RewardError::MiningPeriodEnded);
    }

    let remaining_cap = LIQUIDITY_MINING_USER_CAP.saturating_sub(*user_rewards_earned);
    if remaining_cap == 0 {
        return Err(RewardError::UserCapReached);
    }

    let amount = LIQUIDITY_MINING_REWARD.min(remaining_cap);
    if config.treasury_balance < amount {
        return Err(RewardError::InsufficientTreasury);
    }

    // Enforce the solvency invariant atomically before mutating any state.
    check_solvency_invariant(config, amount)?;

    config.treasury_balance -= amount;
    *user_rewards_earned += amount;

    Ok(LiquidityMiningRewardEarned {
        user,
        amount,
        trades_remaining: ((LIQUIDITY_MINING_USER_CAP - *user_rewards_earned)
            / LIQUIDITY_MINING_REWARD) as u32,
    })
}

/// Read-only preview of the reward that `distribute_liquidity_mining_reward`
/// would produce for the same inputs, without mutating any state.
pub fn quote_liquidity_mining_reward<User: Clone>(
    config: &LiquidityMiningConfig,
    user: User,
    user_rewards_earned: i128,
    now: u64,
) -> Result<LiquidityMiningRewardEarned<User>, RewardError> {
    if !config.liquidity_mining_active {
        return Err(RewardError::MiningInactive);
    }

    let mining_ends_at = config
        .mainnet_launch_timestamp
        .saturating_add(config.mining_period_seconds);
    if now >= mining_ends_at {
        return Err(RewardError::MiningPeriodEnded);
    }

    let remaining_cap = LIQUIDITY_MINING_USER_CAP.saturating_sub(user_rewards_earned);
    if remaining_cap == 0 {
        return Err(RewardError::UserCapReached);
    }

    let amount = LIQUIDITY_MINING_REWARD.min(remaining_cap);
    if config.treasury_balance < amount {
        return Err(RewardError::InsufficientTreasury);
    }

    check_solvency_invariant(config, amount)?;

    Ok(LiquidityMiningRewardEarned {
        user,
        amount,
        trades_remaining: ((LIQUIDITY_MINING_USER_CAP - (user_rewards_earned + amount))
            / LIQUIDITY_MINING_REWARD) as u32,
    })
}

pub fn new_vesting_schedule(
    start_timestamp: u64,
    cliff_seconds: u64,
    duration_seconds: u64,
    total_amount: i128,
) -> VestingSchedule {
    VestingSchedule {
        start_timestamp,
        cliff_seconds,
        duration_seconds,
        total_amount,
        released_amount: 0,
    }
}

pub fn vested_amount(schedule: &VestingSchedule, now: u64) -> i128 {
    if now < schedule.start_timestamp.saturating_add(schedule.cliff_seconds) {
        return 0;
    }

    let elapsed = now.saturating_sub(schedule.start_timestamp);
    if elapsed >= schedule.duration_seconds {
        return schedule.total_amount;
    }

    let vested = (schedule.total_amount as i128)
        .saturating_mul(elapsed as i128)
        / (schedule.duration_seconds as i128);
    vested.min(schedule.total_amount)
}

pub fn releasable_amount(schedule: &VestingSchedule, now: u64) -> i128 {
    vested_amount(schedule, now).saturating_sub(schedule.released_amount)
}

/// Read-only preview of the amount `release_vested_rewards` would release for
/// the same schedule and timestamp, without mutating the schedule.
pub fn quote_releasable_rewards(
    schedule: &VestingSchedule,
    now: u64,
) -> Result<i128, RewardError> {
    if now < schedule.start_timestamp.saturating_add(schedule.cliff_seconds) {
        return Err(RewardError::CliffNotReached);
    }

    let amount = releasable_amount(schedule, now);
    if amount == 0 {
        return Err(RewardError::NothingToRelease);
    }

    Ok(amount)
}

pub fn release_vested_rewards(
    schedule: &mut VestingSchedule,
    now: u64,
) -> Result<i128, RewardError> {
    if now < schedule.start_timestamp.saturating_add(schedule.cliff_seconds) {
        return Err(RewardError::CliffNotReached);
    }

    let amount = releasable_amount(schedule, now);
    if amount == 0 {
        return Err(RewardError::NothingToRelease);
    }

    schedule.released_amount = schedule.released_amount.saturating_add(amount);
    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LiquidityMiningConfig {
        LiquidityMiningConfig {
            liquidity_mining_active: true,
            mainnet_launch_timestamp: 1_700_000_000,
            mining_period_seconds: DEFAULT_MINING_PERIOD_SECONDS,
            treasury_balance: 2_000 * XLM,
            protected_reserves: 0,
        }
    }

    #[test]
    fn rewards_during_mining() {
        let mut config = config();
        let mut earned = 0;

        let event =
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001)
                .unwrap();

        assert_eq!(event.amount, LIQUIDITY_MINING_REWARD);
        assert_eq!(event.trades_remaining, 99);
        assert_eq!(earned, 10 * XLM);
        assert_eq!(config.treasury_balance, 1_990 * XLM);
    }

    #[test]
    fn no_reward_after_mining_period() {
        let mut config = config();
        let mut earned = 0;

        let result = distribute_liquidity_mining_reward(
            &mut config,
            "user-1",
            &mut earned,
            1_700_000_000 + DEFAULT_MINING_PERIOD_SECONDS,
        );

        assert_eq!(result, Err(RewardError::MiningPeriodEnded));
        assert!(!config.liquidity_mining_active);
        assert_eq!(earned, 0);
    }

    #[test]
    fn cap_reached() {
        let mut config = config();
        let mut earned = LIQUIDITY_MINING_USER_CAP;

        let result =
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001);

        assert_eq!(result, Err(RewardError::UserCapReached));
        assert_eq!(config.treasury_balance, 2_000 * XLM);
    }

    #[test]
    fn solvency_allows_withdrawal_at_exact_floor() {
        let mut config = config();
        config.protected_reserves = 1_990 * XLM;

        assert_eq!(withdrawable_amount(&config), 10 * XLM);
        assert_eq!(check_solvency_invariant(&config, 10 * XLM), Ok(()));

        let mut earned = 0;
        let event =
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001)
                .unwrap();

        assert_eq!(event.amount, LIQUIDITY_MINING_REWARD);
        assert_eq!(config.treasury_balance, 1_990 * XLM);
        assert_eq!(config.treasury_balance, config.protected_reserves);
    }

    #[test]
    fn solvency_rejects_withdrawal_below_floor() {
        let mut config = config();
        config.protected_reserves = 1_995 * XLM;

        assert_eq!(
            check_solvency_invariant(&config, 10 * XLM),
            Err(RewardError::SolvencyFloorBreached)
        );

        let mut earned = 0;
        let result =
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001);

        assert_eq!(result, Err(RewardError::SolvencyFloorBreached));
        // State must be left unchanged on violation.
        assert_eq!(config.treasury_balance, 2_000 * XLM);
        assert_eq!(earned, 0);
    }

    #[test]
    fn solvency_allows_withdrawal_after_reserve_replenishment() {
        let mut config = config();
        config.protected_reserves = 1_995 * XLM;

        let mut earned = 0;
        assert_eq!(
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001),
            Err(RewardError::SolvencyFloorBreached)
        );

        // Replenish reserves so the floor is satisfied again.
        config.treasury_balance += 10 * XLM;

        let event =
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001)
                .unwrap();

        assert_eq!(event.amount, LIQUIDITY_MINING_REWARD);
        assert_eq!(config.treasury_balance, 2_000 * XLM);
        assert!(config.treasury_balance >= config.protected_reserves);
    }

    #[test]
    fn nothing_vested_before_cliff() {
        let schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        assert_eq!(vested_amount(&schedule, 1_050), 0);
        assert_eq!(releasable_amount(&schedule, 1_050), 0);
    }

    #[test]
    fn release_blocked_before_cliff() {
        let mut schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        let result = release_vested_rewards(&mut schedule, 1_050);

        assert_eq!(result, Err(RewardError::CliffNotReached));
        assert_eq!(schedule.released_amount, 0);
    }

    #[test]
    fn linear_vesting_after_cliff() {
        let schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        assert_eq!(vested_amount(&schedule, 1_500), 500 * XLM);
        assert_eq!(releasable_amount(&schedule, 1_500), 500 * XLM)
    }
}
