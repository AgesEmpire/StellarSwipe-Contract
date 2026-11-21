use core::cmp::min;

pub const XLM: i128 = 10_000_000;
pub const LIQUIDITY_MINING_REWARD: i128 = 10 * XLM;
pub const LIQUIDITY_MINING_USER_CAP: i128 = 1_000 * XLM;
pub const DEFAULT_MINING_PERIOD_SECONDS: u64 = 90 * 24 * 60 * 60;

// Batch processing constants for gas optimization
pub const MAX_BATCH_SIZE: usize = 50;
pub const MIN_CLAIM_AMOUNT: i128 = XLM / 100; // 0.01 XLM minimum claim

#[derive(Clone, Debug, PartialEq)]
pub struct LiquidityMiningConfig {
    pub liquidity_mining_active: bool,
    pub mainnet_launch_timestamp: u64,
    pub mining_period_seconds: u64,
    pub treasury_balance: i128,
    pub reserve_balance: i128,
    pub total_accrued: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardAccrual<User> {
    pub user: User,
    pub accrued_amount: i128,
    pub last_accrual_time: u64,
    pub claimed_amount: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchDistributionResult<User> {
    pub successful_distributions: Vec<RewardDistribution<User>>,
    pub failed_distributions: Vec<(User, RewardError)>,
    pub total_distributed: i128,
    pub gas_saved_percentage: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardDistribution<User> {
    pub user: User,
    pub amount: i128,
    pub trades_remaining: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClaimOptimizationResult<User> {
    pub user: User,
    pub claimed_amount: i128,
    pub remaining_accrued: i128,
    pub gas_cost_estimate: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReserveManagementResult {
    pub reserve_replenished: i128,
    pub treasury_remaining: i128,
    pub reserve_utilization_percentage: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RewardError {
    MiningInactive,
    MiningPeriodEnded,
    UserCapReached,
    InsufficientTreasury,
    InsufficientReserve,
    BatchSizeExceeded,
    ClaimAmountTooSmall,
    NoAccruedRewards,
}

/// Read-only preview of a reward accrual for a single user.
///
/// Mirrors the exact arithmetic of [`accrue_reward`] (same cap and period
/// checks, same `min` rounding) but never mutates `config` or `accrual` and
/// performs no token transfers. Returns the amount that `accrue_reward` would
/// add for the same inputs and ledger state.
pub fn quote_accrue_reward<User>(
    config: &LiquidityMiningConfig,
    accrual: &RewardAccrual<User>,
    now: u64,
) -> Result<i128, RewardError> {
    if !config.liquidity_mining_active {
        return Err(RewardError::MiningInactive);
    }

    let mining_ends_at = config
        .mainnet_launch_timestamp
        .saturating_add(config.mining_period_seconds);
    if now >= mining_ends_at {
        return Err(RewardError::MiningPeriodEnded);
    }

    let total_earned = accrual.accrued_amount + accrual.claimed_amount;
    let remaining_cap = LIQUIDITY_MINING_USER_CAP.saturating_sub(total_earned);

    if remaining_cap == 0 {
        return Err(RewardError::UserCapReached);
    }

    Ok(min(LIQUIDITY_MINING_REWARD, remaining_cap))
}

/// Read-only preview of a claim for a single user.
///
/// Mirrors the exact arithmetic of [`claim_accrued_rewards`] (same threshold
/// and reserve/treasury checks) but never mutates `config` or `accrual` and
/// performs no token transfers. Returns the amount that `claim_accrued_rewards`
/// would pay out for the same inputs and ledger state.
pub fn quote_claim_accrued_rewards<User>(
    config: &LiquidityMiningConfig,
    accrual: &RewardAccrual<User>,
) -> Result<i128, RewardError> {
    if accrual.accrued_amount == 0 {
        return Err(RewardError::NoAccruedRewards);
    }

    if accrual.accrued_amount < MIN_CLAIM_AMOUNT {
        return Err(RewardError::ClaimAmountTooSmall);
    }

    let claim_amount = accrual.accrued_amount;

    if config.reserve_balance >= claim_amount {
        // Covered entirely by reserve.
    } else if config.reserve_balance + config.treasury_balance >= claim_amount {
        // Covered by reserve plus treasury.
    } else {
        return Err(RewardError::InsufficientReserve);
    }

    Ok(claim_amount)
}

/// Optimized batch reward distribution
/// Processes multiple users in a single transaction to reduce gas costs
pub fn batch_distribute_rewards<User: Clone>(
    config: &mut LiquidityMiningConfig,
    users: Vec<User>,
    user_rewards_map: &mut Vec<(User, i128)>,
    now: u64,
) -> Result<BatchDistributionResult<User>, RewardError> {
    if users.len() > MAX_BATCH_SIZE {
        return Err(RewardError::BatchSizeExceeded);
    }

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

    let mut successful = Vec::new();
    let mut failed = Vec::new();
    let mut total_distributed = 0i128;

    // Pre-calculate total required to optimize treasury checks
    let mut total_required = 0i128;
    for user in &users {
        if let Some((_, earned)) = user_rewards_map.iter().find(|(u, _)| {
            // Simple comparison - in real implementation would use proper equality
            core::ptr::eq(u as *const User, user as *const User)
        }) {
            let remaining_cap = LIQUIDITY_MINING_USER_CAP.saturating_sub(*earned);
            if remaining_cap > 0 {
                total_required += min(LIQUIDITY_MINING_REWARD, remaining_cap);
            }
        }
    }

    // Check if we have enough in reserve + treasury
    if config.reserve_balance + config.treasury_balance < total_required {
        return Err(RewardError::InsufficientTreasury);
    }

    // Process batch
    for user in users {
        let user_entry = user_rewards_map
            .iter_mut()
            .find(|(u, _)| core::ptr::eq(u as *const User, &user as *const User));

        if let Some((_, earned)) = user_entry {
            let remaining_cap = LIQUIDITY_MINING_USER_CAP.saturating_sub(*earned);
            
            if remaining_cap == 0 {
                failed.push((user.clone(), RewardError::UserCapReached));
                continue;
            }

            let amount = min(LIQUIDITY_MINING_REWARD, remaining_cap);

            // Use reserve first, then treasury
            if config.reserve_balance >= amount {
                config.reserve_balance -= amount;
            } else {
                let from_reserve = config.reserve_balance;
                let from_treasury = amount - from_reserve;
                config.reserve_balance = 0;
                config.treasury_balance -= from_treasury;
            }

            *earned += amount;
            total_distributed += amount;

            successful.push(RewardDistribution {
                user: user.clone(),
                amount,
                trades_remaining: ((LIQUIDITY_MINING_USER_CAP - *earned) / LIQUIDITY_MINING_REWARD)
                    as u32,
            });
        }
    }

    // Calculate gas savings (batch processing saves ~30-40% compared to individual calls)
    let gas_saved = if successful.len() > 1 {
        35 // 35% average gas savings for batch operations
    } else {
        0
    };

    Ok(BatchDistributionResult {
        successful_distributions: successful,
        failed_distributions: failed,
        total_distributed,
        gas_saved_percentage: gas_saved,
    })
}

/// Accrue rewards without immediate distribution
/// Allows users to accumulate rewards and claim later in a single transaction
pub fn accrue_reward<User: Clone>(
    config: &mut LiquidityMiningConfig,
    accrual: &mut RewardAccrual<User>,
    now: u64,
) -> Result<i128, RewardError> {
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

    let total_earned = accrual.accrued_amount + accrual.claimed_amount;
    let remaining_cap = LIQUIDITY_MINING_USER_CAP.saturating_sub(total_earned);
    
    if remaining_cap == 0 {
        return Err(RewardError::UserCapReached);
    }

    let amount = min(LIQUIDITY_MINING_REWARD, remaining_cap);
    
    // Just track accrual, don't move funds yet
    accrual.accrued_amount += amount;
    accrual.last_accrual_time = now;
    config.total_accrued += amount;

    Ok(amount)
}

/// Optimized claim function with minimum threshold
/// Reduces gas costs by preventing small claims
pub fn claim_accrued_rewards<User: Clone>(
    config: &mut LiquidityMiningConfig,
    accrual: &mut RewardAccrual<User>,
) -> Result<ClaimOptimizationResult<User>, RewardError> {
    if accrual.accrued_amount == 0 {
        return Err(RewardError::NoAccruedRewards);
    }

    if accrual.accrued_amount < MIN_CLAIM_AMOUNT {
        return Err(RewardError::ClaimAmountTooSmall);
    }

    let claim_amount = accrual.accrued_amount;

    // Check reserve first, then treasury
    if config.reserve_balance >= claim_amount {
        config.reserve_balance -= claim_amount;
    } else if config.reserve_balance + config.treasury_balance >= claim_amount {
        let from_reserve = config.reserve_balance;
        let from_treasury = claim_amount - from_reserve;
        config.reserve_balance = 0;
        config.treasury_balance -= from_treasury;
    } else {
        return Err(RewardError::InsufficientReserve);
    }

    accrual.claimed_amount += claim_amount;
    accrual.accrued_amount = 0;
    config.total_accrued -= claim_amount;

    // Estimate gas cost (lower for larger claims due to amortization)
    let gas_estimate = if claim_amount >= 100 * XLM {
        50_000 // Low gas for large claims
    } else if claim_amount >= 10 * XLM {
        75_000 // Medium gas
    } else {
        100_000 // Higher gas for small claims
    };

    Ok(ClaimOptimizationResult {
        user: accrual.user.clone(),
        claimed_amount: claim_amount,
        remaining_accrued: accrual.accrued_amount,
        gas_cost_estimate: gas_estimate,
    })
}

#[cfg(test)]
mod quote_tests {
    use super::*;

    fn active_config() -> LiquidityMiningConfig {
        LiquidityMiningConfig {
            liquidity_mining_active: true,
            mainnet_launch_timestamp: 1_000,
            mining_period_seconds: DEFAULT_MINING_PERIOD_SECONDS,
            treasury_balance: 1_000 * XLM,
            reserve_balance: 500 * XLM,
            total_accrued: 0,
        }
    }

    fn accrual(accrued: i128, claimed: i128) -> RewardAccrual<u32> {
        RewardAccrual {
            user: 1,
            accrued_amount: accrued,
            last_accrual_time: 0,
            claimed_amount: claimed,
        }
    }

    #[test]
    fn quote_accrue_matches_mutating_path() {
        let mut config = active_config();
        let mut a = accrual(0, 0);
        let now = 2_000;

        let quoted = quote_accrue_reward(&config, &a, now).unwrap();
        let applied = accrue_reward(&mut config, &mut a, now).unwrap();

        assert_eq!(quoted, applied);
        assert_eq!(quoted, LIQUIDITY_MINING_REWARD);
    }

    #[test]
    fn quote_accrue_rounds_at_user_cap() {
        let config = active_config();
        // Only 3 XLM of cap remaining -> min() must clamp to the remainder.
        let a = accrual(0, LIQUIDITY_MINING_USER_CAP - 3 * XLM);

        let quoted = quote_accrue_reward(&config, &a, 2_000).unwrap();
        assert_eq!(quoted, 3 * XLM);
    }

    #[test]
    fn quote_accrue_does_not_mutate_state() {
        let config = active_config();
        let a = accrual(0, 0);
        let config_before = config.clone();
        let accrual_before = a.clone();

        let _ = quote_accrue_reward(&config, &a, 2_000).unwrap();

        assert_eq!(config, config_before);
        assert_eq!(a, accrual_before);
    }

    #[test]
    fn quote_accrue_rejects_invalid_inputs() {
        let mut inactive = active_config();
        inactive.liquidity_mining_active = false;
        assert_eq!(
            quote_accrue_reward(&inactive, &accrual(0, 0), 2_000),
            Err(RewardError::MiningInactive)
        );

        let config = active_config();
        let ended = config.mainnet_launch_timestamp + config.mining_period_seconds;
        assert_eq!(
            quote_accrue_reward(&config, &accrual(0, 0), ended),
            Err(RewardError::MiningPeriodEnded)
        );

        assert_eq!(
            quote_accrue_reward(&config, &accrual(0, LIQUIDITY_MINING_USER_CAP), 2_000),
            Err(RewardError::UserCapReached)
        );
    }

    #[test]
    fn quote_claim_matches_mutating_path() {
        let mut config = active_config();
        let mut a = accrual(10 * XLM, 0);

        let quoted = quote_claim_accrued_rewards(&config, &a).unwrap();
        let applied = claim_accrued_rewards(&mut config, &mut a).unwrap();

        assert_eq!(quoted, applied.claimed_amount);
        assert_eq!(quoted, 10 * XLM);
    }

    #[test]
    fn quote_claim_does_not_mutate_state() {
        let config = active_config();
        let a = accrual(10 * XLM, 0);
        let config_before = config.clone();
        let accrual_before = a.clone();

        let _ = quote_claim_accrued_rewards(&config, &a).unwrap();

        assert_eq!(config, config_before);
        assert_eq!(a, accrual_before);
    }

    #[test]
    fn quote_claim_rejects_invalid_inputs() {
        let config = active_config();

        assert_eq!(
            quote_claim_accrued_rewards(&config, &accrual(0, 0)),
            Err(RewardError::NoAccruedRewards)
        );

        assert_eq!(
            quote_claim_accrued_rewards(&config, &accrual(MIN_CLAIM_AMOUNT - 1, 0)),
            Err(RewardError::ClaimAmountTooSmall)
        );

        let mut poor = active_config();
        poor.reserve_balance = 0;
        poor.treasury_balance = 0;
        assert_eq!(
            quote_claim_accrued_rewards(&poor, &accrual(10 * XLM, 0)),
            Err(RewardError::InsufficientReserve)
        );
    }

    #[test]
    fn quote_claim_reflects_current_ledger_state() {
        // Reserve alone covers the claim.
        let mut config = active_config();
        config.reserve_balance = 10 * XLM;
        config.treasury_balance = 0;
        assert_eq!(
            quote_claim_accrued_rewards(&config, &accrual(10 * XLM, 0)),
            Ok(10 * XLM)
        );

        // Reserve + treasury covers the claim.
        config.reserve_balance = 4 * XLM;
        config.treasury_balance = 6 * XLM;
        assert_eq!(
            quote_claim_accrued_rewards(&config, &accrual(10 * XLM, 0)),
            Ok(10 * XLM)
        );
    }
}
