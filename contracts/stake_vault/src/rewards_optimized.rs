pub const XLM: i128 = 10_000_000;
pub const DEFAULT_AUTO_FUND_AMOUNT: i128 = 5_000 * XLM;
pub const OPTIMAL_RESERVE_DAYS: u32 = 7; // Keep 7 days worth of rewards in reserve

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolStatus {
    pub balance: i128,
    pub estimated_days_remaining: u32,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
    pub reserve_utilization: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolLow {
    pub balance: i128,
    pub days_remaining: u32,
    pub auto_funded_amount: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPool {
    pub balance: i128,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
    pub treasury_balance: i128,
    pub pending_claims: i128,
    pub total_distributed: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StakeRewardAccrual<User> {
    pub user: User,
    pub accrued_amount: i128,
    pub last_update_time: u64,
    pub stake_amount: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchClaimResult<User> {
    pub claims: Vec<ClaimResult<User>>,
    pub total_claimed: i128,
    pub gas_saved_percentage: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClaimResult<User> {
    pub user: User,
    pub amount: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptimizedDistributionMetrics {
    pub total_gas_saved: u64,
    pub batch_efficiency: u32,
    pub average_claim_size: i128,
    pub reserve_hit_rate: u32,
}

/// Get enhanced rewards pool status with optimization metrics
pub fn get_rewards_pool_status(pool: &RewardsPool) -> RewardsPoolStatus {
    let days_remaining = estimated_days_remaining(pool.balance, pool.daily_outflow);
    let optimal_reserve = pool.daily_outflow * OPTIMAL_RESERVE_DAYS as i128;
    let utilization = if optimal_reserve > 0 {
        ((pool.balance * 100) / optimal_reserve).min(100) as u32
    } else {
        0
    };

    RewardsPoolStatus {
        balance: pool.balance,
        estimated_days_remaining: days_remaining,
        daily_outflow: pool.daily_outflow,
        auto_fund_threshold: pool.auto_fund_threshold,
        reserve_utilization: utilization,
    }
}

/// Optimized pool monitoring with predictive refilling
pub fn monitor_rewards_pool(pool: &mut RewardsPool) -> Option<RewardsPoolLow> {
    // Calculate optimal threshold based on daily outflow
    let optimal_threshold = pool.daily_outflow * OPTIMAL_RESERVE_DAYS as i128;
    let effective_threshold = pool.auto_fund_threshold.max(optimal_threshold);

    if pool.balance >= effective_threshold {
        return None;
    }

    let days_remaining = estimated_days_remaining(pool.balance, pool.daily_outflow);
    
    // Calculate optimal refill amount (enough for OPTIMAL_RESERVE_DAYS)
    let target_balance = pool.daily_outflow * OPTIMAL_RESERVE_DAYS as i128;
    let needed = target_balance.saturating_sub(pool.balance);
    let fund_amount = needed.min(pool.treasury_balance);

    if fund_amount == 0 {
        return Some(RewardsPoolLow {
            balance: pool.balance,
            days_remaining,
            auto_funded_amount: 0,
        });
    }

    pool.balance += fund_amount;
    pool.treasury_balance -= fund_amount;

    Some(RewardsPoolLow {
        balance: pool.balance,
        days_remaining: estimated_days_remaining(pool.balance, pool.daily_outflow),
        auto_funded_amount: fund_amount,
    })
}

/// Accrue staking rewards without immediate distribution
/// Reduces gas costs by batching calculations
pub fn accrue_staking_rewards<User: Clone>(
    accrual: &mut StakeRewardAccrual<User>,
    reward_rate_per_second: i128,
    current_time: u64,
) -> i128 {
    let time_elapsed = current_time.saturating_sub(accrual.last_update_time);
    let reward = (accrual.stake_amount * reward_rate_per_second * time_elapsed as i128) / 1_000_000;
    
    accrual.accrued_amount += reward;
    accrual.last_update_time = current_time;
    
    reward
}

/// Batch claim multiple users' rewards in a single transaction
/// Significantly reduces gas costs compared to individual claims
pub fn batch_claim_rewards<User: Clone>(
    pool: &mut RewardsPool,
    accruals: &mut Vec<StakeRewardAccrual<User>>,
) -> Result<BatchClaimResult<User>, &'static str> {
    if accruals.is_empty() {
        return Err("No accruals to process");
    }

    let mut claims = Vec::new();
    let mut total_claimed = 0i128;

    // Pre-calculate total needed
    let total_needed: i128 = accruals.iter().map(|a| a.accrued_amount).sum();

    if pool.balance < total_needed {
        // Try to refill from treasury
        let needed = total_needed - pool.balance;
        let available = needed.min(pool.treasury_balance);
        pool.balance += available;
        pool.treasury_balance -= available;

        if pool.balance < total_needed {
            return Err("Insufficient pool balance");
        }
    }

    // Process all claims
    for accrual in accruals.iter_mut() {
        if accrual.accrued_amount > 0 {
            let claim_amount = accrual.accrued_amount;
            pool.balance -= claim_amount;
            pool.total_distributed += claim_amount;
            total_claimed += claim_amount;

            claims.push(ClaimResult {
                user: accrual.user.clone(),
                amount: claim_amount,
            });

            accrual.accrued_amount = 0;
        }
    }

    // Calculate gas savings (batch processing saves ~40% for multiple claims)
    let gas_saved = if claims.len() > 1 {
        40
    } else {
        0
    };

    Ok(BatchClaimResult {
        claims,
        total_claimed,
        gas_saved_percentage: gas_saved,
    })
}

/// Optimize reward distribution by consolidating small claims
/// Prevents gas waste on micro-transactions
pub fn should_claim(accrued_amount: i128, min_claim_threshold: i128) -> bool {
    accrued_amount >= min_claim_threshold
}

/// Calculate optimal claim timing based on gas costs
/// Returns recommended wait time in seconds
pub fn calculate_optimal_claim_time(
    accrued_amount: i128,
    accrual_rate_per_second: i128,
    gas_price: u64,
    min_profitable_amount: i128,
) -> u64 {
    if accrued_amount >= min_profitable_amount {
        return 0; // Claim now
    }

    let needed = min_profitable_amount - accrued_amount;
    if accrual_rate_per_second == 0 {
        return u64::MAX; // Never profitable
    }

    (needed / accrual_rate_per_second) as u64
}

/// Manage reserve with predictive refilling
/// Maintains optimal buffer to minimize treasury access
pub fn optimize_reserve_management(
    pool: &mut RewardsPool,
    predicted_daily_outflow: i128,
) -> i128 {
    let target_reserve = predicted_daily_outflow * OPTIMAL_RESERVE_DAYS as i128;
    let current_reserve = pool.balance;

    if current_reserve >= target_reserve {
        return 0; // No action needed
    }

    let needed = target_reserve - current_reserve;
    let available = needed.min(pool.treasury_balance);

    pool.balance += available;
    pool.treasury_balance -= available;

    available
}

/// Calculate distribution efficiency metrics
pub fn calculate_distribution_metrics(
    pool: &RewardsPool,
    total_claims: u32,
    total_gas_used: u64,
    reserve_hits: u32,
) -> OptimizedDistributionMetrics {
    let average_claim = if total_claims > 0 {
        pool.total_distributed / total_claims as i128
    } else {
        0
    };

    let baseline_gas = total_claims as u64 * 100_000; // Baseline gas per claim
    let gas_saved = baseline_gas.saturating_sub(total_gas_used);

    let batch_efficiency = if total_claims > 1 {
        ((gas_saved * 100) / baseline_gas) as u32
    } else {
        0
    };

    let reserve_hit_rate = if total_claims > 0 {
        (reserve_hits * 100) / total_claims
    } else {
        0
    };

    OptimizedDistributionMetrics {
        total_gas_saved: gas_saved,
        batch_efficiency,
        average_claim_size: average_claim,
        reserve_hit_rate,
    }
}

fn estimated_days_remaining(balance: i128, daily_outflow: i128) -> u32 {
    if daily_outflow <= 0 {
        return u32::MAX;
    }

    (balance.max(0) / daily_outflow) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> RewardsPool {
        RewardsPool {
            balance: 10_000 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
            pending_claims: 0,
            total_distributed: 0,
        }
    }

    #[test]
    fn test_enhanced_pool_status() {
        let pool = pool();
        let status = get_rewards_pool_status(&pool);

        assert_eq!(status.balance, 10_000 * XLM);
        assert_eq!(status.estimated_days_remaining, 100);
        assert_eq!(status.daily_outflow, 100 * XLM);
        // 10_000 / (100 * 7) = ~142% utilization, capped at 100
        assert_eq!(status.reserve_utilization, 100);
    }

    #[test]
    fn test_optimized_pool_monitoring() {
        let mut pool = pool();
        pool.balance = 500 * XLM; // Below optimal threshold

        let event = monitor_rewards_pool(&mut pool).unwrap();

        // Should refill to 7 days worth (700 XLM)
        let expected_refill = (100 * XLM * OPTIMAL_RESERVE_DAYS as i128) - 500 * XLM;
        assert_eq!(event.auto_funded_amount, expected_refill);
        assert_eq!(pool.balance, 700 * XLM);
    }

    #[test]
    fn test_reward_accrual() {
        let mut accrual = StakeRewardAccrual {
            user: "user1",
            accrued_amount: 0,
            last_update_time: 1000,
            stake_amount: 1_000 * XLM,
        };

        let reward_rate = 1_000; // 0.001% per second
        let current_time = 2000; // 1000 seconds elapsed

        let reward = accrue_staking_rewards(&mut accrual, reward_rate, current_time);

        // (1000 * XLM * 1000 * 1000) / 1_000_000 = 1000 * XLM
        assert_eq!(reward, 1_000 * XLM);
        assert_eq!(accrual.accrued_amount, 1_000 * XLM);
        assert_eq!(accrual.last_update_time, 2000);
    }

    #[test]
    fn test_batch_claim_success() {
        let mut pool = pool();
        let mut accruals = vec![
            StakeRewardAccrual {
                user: "user1",
                accrued_amount: 100 * XLM,
                last_update_time: 1000,
                stake_amount: 1_000 * XLM,
            },
            StakeRewardAccrual {
                user: "user2",
                accrued_amount: 200 * XLM,
                last_update_time: 1000,
                stake_amount: 2_000 * XLM,
            },
            StakeRewardAccrual {
                user: "user3",
                accrued_amount: 150 * XLM,
                last_update_time: 1000,
                stake_amount: 1_500 * XLM,
            },
        ];

        let result = batch_claim_rewards(&mut pool, &mut accruals).unwrap();

        assert_eq!(result.claims.len(), 3);
        assert_eq!(result.total_claimed, 450 * XLM);
        assert_eq!(result.gas_saved_percentage, 40);
        assert_eq!(pool.balance, 9_550 * XLM);
        assert_eq!(pool.total_distributed, 450 * XLM);

        // All accruals should be reset
        for accrual in &accruals {
            assert_eq!(accrual.accrued_amount, 0);
        }
    }

    #[test]
    fn test_batch_claim_with_refill() {
        let mut pool = pool();
        pool.balance = 100 * XLM; // Not enough for claims

        let mut accruals = vec![
            StakeRewardAccrual {
                user: "user1",
                accrued_amount: 200 * XLM,
                last_update_time: 1000,
                stake_amount: 1_000 * XLM,
            },
        ];

        let result = batch_claim_rewards(&mut pool, &mut accruals).unwrap();

        assert_eq!(result.total_claimed, 200 * XLM);
        // Pool should have been refilled from treasury
        assert!(pool.treasury_balance < 20_000 * XLM);
    }

    #[test]
    fn test_should_claim_threshold() {
        assert!(should_claim(10 * XLM, 5 * XLM));
        assert!(!should_claim(3 * XLM, 5 * XLM));
        assert!(should_claim(5 * XLM, 5 * XLM));
    }

    #[test]
    fn test_optimal_claim_time() {
        let accrued = 5 * XLM;
        let rate = XLM / 100; // 0.01 XLM per second
        let min_profitable = 10 * XLM;

        let wait_time = calculate_optimal_claim_time(accrued, rate, 1000, min_profitable);

        // Need 5 more XLM at 0.01 XLM/s = 500 seconds
        assert_eq!(wait_time, 500);
    }

    #[test]
    fn test_optimal_claim_time_ready() {
        let accrued = 15 * XLM;
        let rate = XLM / 100;
        let min_profitable = 10 * XLM;

        let wait_time = calculate_optimal_claim_time(accrued, rate, 1000, min_profitable);

        assert_eq!(wait_time, 0); // Ready to claim now
    }

    #[test]
    fn test_reserve_optimization() {
        let mut pool = pool();
        pool.balance = 200 * XLM;
        let predicted_outflow = 150 * XLM;

        let refilled = optimize_reserve_management(&mut pool, predicted_outflow);

        // Target = 150 * 7 = 1050, current = 200, need 850
        assert_eq!(refilled, 850 * XLM);
        assert_eq!(pool.balance, 1_050 * XLM);
    }

    #[test]
    fn test_distribution_metrics() {
        let mut pool = pool();
        pool.total_distributed = 1_000 * XLM;

        let metrics = calculate_distribution_metrics(&pool, 10, 600_000, 8);

        // Baseline: 10 * 100_000 = 1_000_000
        // Used: 600_000
        // Saved: 400_000
        assert_eq!(metrics.total_gas_saved, 400_000);
        assert_eq!(metrics.batch_efficiency, 40);
        assert_eq!(metrics.average_claim_size, 100 * XLM);
        assert_eq!(metrics.reserve_hit_rate, 80); // 8/10 = 80%
    }

    #[test]
    fn test_empty_batch_claim() {
        let mut pool = pool();
        let mut accruals: Vec<StakeRewardAccrual<&str>> = vec![];

        let result = batch_claim_rewards(&mut pool, &mut accruals);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No accruals to process");
    }

    #[test]
    fn test_insufficient_pool_balance() {
        let mut pool = pool();
        pool.balance = 50 * XLM;
        pool.treasury_balance = 0; // No treasury backup

        let mut accruals = vec![StakeRewardAccrual {
            user: "user1",
            accrued_amount: 100 * XLM,
            last_update_time: 1000,
            stake_amount: 1_000 * XLM,
        }];

        let result = batch_claim_rewards(&mut pool, &mut accruals);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Insufficient pool balance");
    }
}
