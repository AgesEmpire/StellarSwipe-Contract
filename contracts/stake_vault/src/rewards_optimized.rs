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

/// A reconciliation checkpoint capturing the reward state of the stake vault
/// at a point in time.
///
/// The checkpoint records the accumulated deposits, the elapsed time since the
/// vault started accruing, and the total amount distributed so far. Comparing
/// consecutive checkpoints (or a checkpoint against live pool state) lets
/// callers diagnose reward drift.
#[derive(Clone, Debug, PartialEq)]
pub struct RewardCheckpoint {
    /// Total rewards deposited into the vault up to this checkpoint.
    pub total_deposits: i128,
    /// Unix timestamp (seconds) at which this checkpoint was taken.
    pub timestamp: u64,
    /// Seconds elapsed since the vault began accruing rewards.
    pub elapsed_time: u64,
    /// Total rewards distributed (claimed) up to this checkpoint.
    pub total_distributed: i128,
    /// Rewards accrued but not yet distributed at this checkpoint.
    pub pending_rewards: i128,
}

/// The kind of drift detected during reconciliation.
#[derive(Clone, Debug, PartialEq)]
pub enum RewardDrift {
    /// Rewards that should have been distributed are missing.
    Missing,
    /// Rewards were distributed more than once.
    Duplicated,
    /// More rewards were distributed than were ever deposited/accrued.
    Excess,
}

/// Result of reconciling a checkpoint against expected reward state.
#[derive(Clone, Debug, PartialEq)]
pub struct ReconciliationResult {
    /// Whether the checkpoint is consistent with the expected state.
    pub balanced: bool,
    /// The signed drift amount (expected - actual). Positive means rewards are
    /// missing; negative means excess rewards were distributed.
    pub drift: i128,
    /// The specific drift categories detected.
    pub drifts: Vec<RewardDrift>,
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

/// Build a reconciliation checkpoint from the current pool state.
///
/// The checkpoint captures accumulated deposits, elapsed time since the vault
/// started accruing, and the total distributed amount so drift can be
/// diagnosed later.
pub fn create_reward_checkpoint(
    pool: &RewardsPool,
    total_deposits: i128,
    start_time: u64,
    current_time: u64,
) -> RewardCheckpoint {
    let elapsed_time = current_time.saturating_sub(start_time);
    let pending_rewards = total_deposits
        .saturating_sub(pool.total_distributed)
        .max(0);

    RewardCheckpoint {
        total_deposits,
        timestamp: current_time,
        elapsed_time,
        total_distributed: pool.total_distributed,
        pending_rewards,
    }
}

/// Reconcile a checkpoint against the expected reward state.
///
/// `expected_distributed` is the amount that should have been distributed given
/// the accumulated deposits and elapsed time. The function detects:
/// - missing rewards (expected > actual),
/// - duplicated rewards (actual exceeds the expected distribution for the
///   elapsed time while deposits still cover it),
/// - excess rewards (actual distributed exceeds total deposits).
pub fn reconcile_reward_checkpoint(
    checkpoint: &RewardCheckpoint,
    expected_distributed: i128,
) -> ReconciliationResult {
    let actual = checkpoint.total_distributed;
    let mut drifts = Vec::new();

    if actual > checkpoint.total_deposits {
        drifts.push(RewardDrift::Excess);
    }

    if actual > expected_distributed && actual <= checkpoint.total_deposits {
        drifts.push(RewardDrift::Duplicated);
    }

    if actual < expected_distributed {
        drifts.push(RewardDrift::Missing);
    }

    let drift = expected_distributed.saturating_sub(actual);

    ReconciliationResult {
        balanced: drifts.is_empty(),
        drift,
        drifts,
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
        // Try to r

/* … truncated 3421 chars — edit only what you need near the top … */
