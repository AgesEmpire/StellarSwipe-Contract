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

/// A per-provider (or per-strategy) reward vesting schedule.
///
/// Rewards are released linearly from `start_time` over `duration` seconds.
/// Any amount not yet released by the schedule is considered unvested and
/// remains inaccessible until the corresponding release time is reached.
#[derive(Clone, Debug, PartialEq)]
pub struct VestingSchedule {
    /// Total amount of rewards subject to vesting.
    pub total_amount: i128,
    /// Amount already released (claimed) from the schedule.
    pub released_amount: i128,
    /// Unix timestamp (seconds) at which vesting begins.
    pub start_time: u64,
    /// Length of the vesting period in seconds.
    pub duration: u64,
}

/// Result of a vesting release attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct VestingRelease {
    /// Amount that was released in this call.
    pub released: i128,
    /// Amount still locked by the schedule after this release.
    pub remaining_locked: i128,
}

/// Create a new vesting schedule for a provider's rewards.
///
/// `duration` of zero means the full amount is immediately vested.
pub fn create_vesting_schedule(
    total_amount: i128,
    start_time: u64,
    duration: u64,
) -> VestingSchedule {
    VestingSchedule {
        total_amount: total_amount.max(0),
        released_amount: 0,
        start_time,
        duration,
    }
}

/// Deterministically compute the total amount vested at `current_time`.
///
/// Vesting is linear: `total_amount * elapsed / duration`, capped at
/// `total_amount`. Before `start_time` nothing is vested; once the full
/// duration has elapsed the entire amount is vested.
pub fn vested_amount(schedule: &VestingSchedule, current_time: u64) -> i128 {
    if schedule.total_amount <= 0 {
        return 0;
    }

    if current_time <= schedule.start_time {
        return 0;
    }

    if schedule.duration == 0 {
        return schedule.total_amount;
    }

    let elapsed = current_time - schedule.start_time;
    if elapsed >= schedule.duration {
        return schedule.total_amount;
    }

    (schedule.total_amount * elapsed as i128) / schedule.duration as i128
}

/// Amount still locked (unvested) at `current_time`.
///
/// Unvested balances remain inaccessible until the schedule releases them.
pub fn unvested_amount(schedule: &VestingSchedule, current_time: u64) -> i128 {
    let vested = vested_amount(schedule, current_time);
    vested.saturating_sub(schedule.released_amount).max(0)
}

/// Amount currently claimable from the schedule at `current_time`.
///
/// This is the vested amount minus whatever has already been released.
pub fn claimable_amount(schedule: &VestingSchedule, current_time: u64) -> i128 {
    let vested = vested_amount(schedule, current_time);
    (vested - schedule.released_amount).max(0)
}

/// Release the currently vested portion of a schedule.
///
/// Only the amount that has vested by `current_time` is released; the
/// remaining balance stays locked until later release events. Returns the
/// amount released and the amount still locked.
pub fn release_vested_rewards(
    schedule: &mut VestingSchedule,
    current_time: u64,
) -> VestingRelease {
    let claimable = claimable_amount(schedule, current_time);

    if claimable > 0 {
        schedule.released_amount += claimable;
    }

    VestingRelease {
        released: claimable,
        remaining_locked: unvested_amount(schedule, current_time),
    }
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

fn estimate