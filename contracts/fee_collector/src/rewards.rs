//! Reward distribution and fee-collector balance management.
//!
//! This module handles the accounting of collected fees and provides a
//! tightly-scoped, auditable path for sweeping negligible "dust" balances
//! out of the collector.

use soroban_sdk::{contractevent, contracttype, Address, Env};

/// Maximum balance (in stroops) that is considered "dust" and therefore
/// eligible for the auditable sweep path. Any balance greater than or equal
/// to this threshold is a normal fee balance and MUST NOT be sweepable
/// through [`sweep_dust`].
pub const DUST_THRESHOLD: i128 = 1_000;

/// Default rebate cap: 80% of epoch fees may be distributed as rebates.
pub const DEFAULT_MAX_REBATE_BPS: u32 = 8_000;
/// Hard upper bound for the configurable rebate cap (100%).
pub const MAX_REBATE_BPS_LIMIT: u32 = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub struct LiquidityMiningConfig {
    pub liquidity_mining_active: bool,
    pub mainnet_launch_timestamp: u64,
    pub mining_period_seconds: u64,
    pub treasury_balance: i128,
    /// Protected reserves that must remain untouched by withdrawals/payouts.
    pub protected_reserves: i128,
}

/// Storage key for the collector's current balance.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Balance,
}

/// Emitted whenever a dust sweep is performed, recording the destination,
/// the swept amount, and the authorizing caller for audit purposes.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DustSwept {
    #[topic]
    pub caller: Address,
    pub destination: Address,
    pub amount: i128,
}

/// Returns the collector's current balance.
pub fn balance(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::Balance).unwrap_or(0)
}

/// Sets the collector's current balance.
pub fn set_balance(env: &Env, amount: i128) {
    env.storage().instance().set(&DataKey::Balance, &amount);
}

#[derive(Clone, Debug, PartialEq)]
pub enum RewardError {
    MiningInactive,
    MiningPeriodEnded,
    UserCapReached,
    InsufficientTreasury,
    CliffNotReached,
    NothingToRelease,
    InvalidRebateBps,
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

/// Emitted when the per-epoch rebate cap is applied and claims are scaled down.
#[derive(Clone, Debug, PartialEq)]
pub struct RebateCapApplied {
    pub epoch: u64,
    pub requested: i128,
    pub distributed: i128,
}

/// A single pending rebate claim for an epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct RebateClaim<User> {
    pub claimant: User,
    pub amount: i128,
}

/// Result of distributing rebates for an epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct RebateDistribution<User> {
    pub payouts: Vec<RebateClaim<User>>,
    pub cap_event: Option<RebateCapApplied>,
}

/// Admin-only setter for the per-epoch rebate cap. Rejects values above 10000 bps.
pub fn set_max_rebate_bps(current: &mut u32, bps: u32) -> Result<(), RewardError> {
    if bps > MAX_REBATE_BPS_LIMIT {
        return Err(RewardError::InvalidRebateBps);
    }
    *current = bps;
    Ok(())
}

/// Distributes pending rebate claims for an epoch, capping the total at
/// `max_rebate_bps` of `epoch_fees`. When the cap triggers, each claim is scaled
/// proportionally so the payouts sum to the cap, and a `RebateCapApplied` event
/// is returned.
pub fn distribute_rebates<User: Clone>(
    epoch: u64,
    epoch_fees: i128,
    max_rebate_bps: u32,
    claims: &[RebateClaim<User>],
) -> RebateDistribution<User> {
    let total_requested: i128 = claims.iter().map(|c| c.amount).sum();

    if total_requested <= 0 {
        return RebateDistribution {
            payouts: Vec::new(),
            cap_event: None,
        };
    }

    let cap = epoch_fees
        .saturating_mul(max_rebate_bps as i128)
        / MAX_REBATE_BPS_LIMIT as i128;

    if total_requested <= cap {
        return RebateDistribution {
            payouts: claims.to_vec(),
            cap_event: None,
        };
    }

    let mut payouts: Vec<RebateClaim<User>> = Vec::with_capacity(claims.len());
    let mut distributed: i128 = 0;
    for claim in claims {
        let adjusted = claim
            .amount
            .saturating_mul(cap)
            / total_requested;
        distributed = distributed.saturating_add(adjusted);
        payouts.push(RebateClaim {
            claimant: claim.claimant.clone(),
            amount: adjusted,
        });
    }

    RebateDistribution {
        payouts,
        cap_event: Some(RebateCapApplied {
            epoch,
            requested: total_requested,
            distributed,
        }),
    }
}

/// Emitted whenever a dust sweep is performed, recording the destination,
/// the swept amount, and the authorizing caller for audit purposes.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DustSwept {
    #[topic]
    pub caller: Address,
    pub destination: Address,
    pub amount: i128,
}

/// Returns the collector's current balance.
pub fn balance(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::Balance).unwrap_or(0)
}

/// Sweeps a negligible dust balance to an explicit destination.
///
/// This is a controlled, auditable path: it may only be invoked by the
/// configured authority, only for balances strictly below [`DUST_THRESHOLD`],
/// and only to a non-zero destination for a positive amount not exceeding the
/// current balance. Ordinary fee balances cannot be moved through this path.
///
/// # Panics
/// - If `caller` is not the configured authority.
/// - If `destination` is the collector itself (inv
}

/// Sweeps a negligible dust balance to an explicit destination.
///
/// This is a controlled, auditable path: it may only be invoked by the
/// configured authority, only for balances strictly below [`DUST_THRESHOLD`],
/// and only to a non-zero destination for a positive amount not exceeding the
/// current balance. Ordinary fee balances cannot be moved through this path.
///
/// # Panics
/// - If `caller` is not the configured authority.
/// - If `destination` is the collector itself (invalid destination).
/// - If `amount` is not positive.
/// - If `amount` exceeds the current balance.
/// - If `amount` is not strictly below [`DUST_THRESHOLD`].
/// - If the current balance is not strictly below [`DUST_THRESHOLD`].
pub fn sweep_dust(env: &Env, caller: Address, destination: Address, amount: i128) {
    caller.require_auth();

    let authority: Address = env
        .storage()
        .instance()
        .get(&DataKey::Authority)
        .expect("authority not set");
    if caller != authority {
        panic!("unauthorized: caller is not the authority");
    }

    if destination == env.current_contract_address() {
        panic!("invalid destination");
    }
    if amount <= 0 {
        panic!("amount must be positive");
    }

    // Enforce the solvency invariant atomically before mutating any state.
    check_solvency_invariant(config, amount)?;

    config.treasury_balance -= amount;
    *user_rewards_earned += amount;

    let current = balance(env);
    if current >= DUST_THRESHOLD {
        panic!("balance is not dust");
    }
    if amount >= DUST_THRESHOLD {
        panic!("amount is not dust");
    }
    if amount > current {
        panic!("amount exceeds balance");
    }

    set_balance(env, current - amount);

    DustSwept {
        caller,
        destination,
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
    .publish(env);
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

    #[test]
    fn rebates_below_cap_pay_in_full() {
        let claims = vec![
            RebateClaim { claimant: "a", amount: 100 },
            RebateClaim { claimant: "b", amount: 200 },
        ];

        let result = distribute_rebates(1, 1_000, DEFAULT_MAX_REBATE_BPS, &claims);

        assert_eq!(result.cap_event, None);
        assert_eq!(result.payouts, claims);
    }

    #[test]
    fn rebates_above_cap_are_scaled_proportionally() {
        let claims = vec![
            RebateClaim { claimant: "a", amount: 600 },
            RebateClaim { claimant: "b", amount: 400 },
        ];

        // cap = 1000 * 8000 / 10000 = 800
        let result = distribute_rebates(7, 1_000, DEFAULT_MAX_REBATE_BPS, &claims);

        assert_eq!(result.payouts[0].amount, 480);
        assert_eq!(result.payouts[1].amount, 320);
        let total: i128 = result.payouts.iter().map(|p| p.amount).sum();
        assert_eq!(total, 800);

        let event = result.cap_event.unwrap();
        assert_eq!(event.epoch, 7);
        assert_eq!(event.requested, 1_000);
        assert_eq!(event.distributed, 800);
    }

    #[test]
    fn single_claimant_at_exactly_the_cap() {
        let claims = vec![RebateClaim { claimant: "a", amount: 800 }];

        let result = distribute_rebates(3, 1_000, DEFAULT_MAX_REBATE_BPS, &claims);

        assert_eq!(result.cap_event, None);
        assert_eq!(result.payouts[0].amount, 800);
    }

    #[test]
    fn set_max_rebate_bps_is_bounded() {
        let mut bps = DEFAULT_MAX_REBATE_BPS;

        assert_eq!(set_max_rebate_bps(&mut bps, 5_000), Ok(()));
        assert_eq!(bps, 5_000);

        assert_eq!(
            set_max_rebate_bps(&mut bps, 10_001),
            Err(RewardError::InvalidRebateBps)
        );
        assert_eq!(bps, 5_000);

        assert_eq!(set_max_rebate_bps(&mut bps, MAX_REBATE_BPS_LIMIT), Ok(()));
        assert_eq!(bps, MAX_REBATE_BPS_LIMIT);
    }
}

}
