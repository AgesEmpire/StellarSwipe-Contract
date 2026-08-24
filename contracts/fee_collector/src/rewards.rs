pub const XLM: i128 = 10_000_000;
pub const LIQUIDITY_MINING_REWARD: i128 = 10 * XLM;
pub const LIQUIDITY_MINING_USER_CAP: i128 = 1_000 * XLM;
pub const DEFAULT_MINING_PERIOD_SECONDS: u64 = 90 * 24 * 60 * 60;

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
pub enum RewardError {
    MiningInactive,
    MiningPeriodEnded,
    UserCapReached,
    InsufficientTreasury,
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
}
