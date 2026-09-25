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
        assert_eq!(releasable_amount(&schedule, 1_500), 500 * XLM);
    }

    #[test]
    fn fully_vested_after_duration() {
        let schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        assert_eq(vested_amount(&schedule, 2_000), 1_000 * XLM);
        assert_eq!(vested_amount(&schedule, 5_000), 1_000 * XLM);
    }

    #[test]
    fn release_is_incremental_and_deterministic() {
        let mut schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        let first = release_vested_rewards(&mut schedule, 1_500).unwrap();
        assert_eq!(first, 500 * XLM);
        assert_eq!(schedule.released_amount, 500 * XLM);

        let second = release_vested_rewards(&mut schedule, 1_750).unwrap();
        assert_eq!(second, 250 * XLM);
        assert_eq!(schedule.released_amount, 750 * XLM);

        assert_eq!(release_vested_rewards(&mut schedule, 1_750), Err(RewardError::NothingToRelease));
    }

    #[test]
    fn quote_matches_distribute_and_does_not_mutate() {
        let mut config = config();
        let mut earned = 0;

        let quote =
            quote_liquidity_mining_reward(&config, "user-1", earned, 1_700_000_001).unwrap();
        let event =
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001)
                .unwrap();

        assert_eq!(quote, event);
    }

    #[test]
    fn quote_does_not_mutate_ledger_state() {
        let config = config();
        let earned = 0;

        let _ = quote_liquidity_mining_reward(&config, "user-1", earned, 1_700_000_001).unwrap();

        assert_eq!(config.treasury_balance, 2_000 * XLM);
        assert!(config.liquidity_mining_active);
        assert_eq!(earned, 0);
    }

    #[test]
    fn quote_rounds_partial_cap_like_distribute() {
        let mut config = config();
        let mut earned = LIQUIDITY_MINING_USER_CAP - (LIQUIDITY_MINING_REWARD / 2);

        let quote =
            quote_liquidity_mining_reward(&config, "user-1", earned, 1_700_000_001).unwrap();
        let event =
            distribute_liquidity_mining_reward(&mut config, "user-1", &mut earned, 1_700_000_001)
                .unwrap();

        assert_eq!(quote.amount, LIQUIDITY_MINING_REWARD / 2);
        assert_eq!(quote, event);
    }

    #[test]
    fn quote_rejects_invalid_inputs() {
        let mut inactive = config();
        inactive.liquidity_mining_active = false;
        assert_eq!(
            quote_liquidity_mining_reward(&inactive, "user-1", 0, 1_700_000_001),
            Err(RewardError::MiningInactive)
        );

        let ended = config();
        assert_eq!(
            quote_liquidity_mining_reward(
                &ended,
                "user-1",
                0,
                1_700_000_000 + DEFAULT_MINING_PERIOD_SECONDS,
            ),
            Err(RewardError::MiningPeriodEnded)
        );

        let capped = config();
        assert_eq!(
            quote_liquidity_mining_reward(&capped, "user-1", LIQUIDITY_MINING_USER_CAP, 1_700_000_001),
            Err(RewardError::UserCapReached)
        );

        let mut poor = config();
        poor.treasury_balance = LIQUIDITY_MINING_REWARD - 1;
        assert_eq!(
            quote_liquidity_mining_reward(&poor, "user-1", 0, 1_700_000_001),
            Err(RewardError::InsufficientTreasury)
        );
    }

    #[test]
    fn quote_releasable_matches_release_and_does_not_mutate() {
        let mut schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        let quote = quote_releasable_rewards(&schedule, 1_500).unwrap();
        let released = release_vested_rewards(&mut schedule, 1_500).unwrap();

        assert_eq!(quote, released);
        assert_eq!(quote, 500 * XLM);
    }

    #[test]
    fn quote_releasable_does_not_mutate_schedule() {
        let schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        let _ = quote_releasable_rewards(&schedule, 1_500).unwrap();

        assert_eq!(schedule.released_amount, 0);
    }

    #[test]
    fn quote_releasable_rejects_invalid_inputs() {
        let schedule = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);

        assert_eq!(
            quote_releasable_rewards(&schedule, 1_050),
            Err(RewardError::CliffNotReached)
        );

        let mut fully_released = new_vesting_schedule(1_000, 100, 1_000, 1_000 * XLM);
        fully_released.released_amount = 1_000 * XLM;
        assert_eq!(
            quote_releasable_rewards(&fully_released, 2_000),
            Err(RewardError::NothingToRelease)
        );
    }
}
