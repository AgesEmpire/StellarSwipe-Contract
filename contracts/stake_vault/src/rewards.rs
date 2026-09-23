pub const XLM: i128 = 10_000_000;
pub const DEFAULT_AUTO_FUND_AMOUNT: i128 = 5_000 * XLM;

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolStatus {
    pub balance: i128,
    pub estimated_days_remaining: u32,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolLow {
    pub balance: i128,
    pub days_remaining: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPool {
    pub balance: i128,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
    pub treasury_balance: i128,
}

/// Minimum balance policies enforced on partial withdrawals.
///
/// `user_minimum` is the reserve that must remain in a user's account after
/// any withdraw, while `strategy_minimum` is the reserve that must remain in
/// the strategy position. Both are inclusive lower bounds: a withdraw that
/// would leave a balance strictly below the relevant minimum is rejected.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimumBalancePolicy {
    pub user_minimum: i128,
    pub strategy_minimum: i128,
}

impl MinimumBalancePolicy {
    pub fn new(user_minimum: i128, strategy_minimum: i128) -> Self {
        Self {
            user_minimum,
            strategy_minimum,
        }
    }

    /// Validate a partial withdraw before any state is mutated.
    ///
    /// Returns `Ok(())` when the withdraw preserves both the user-level and
    /// strategy-level minimums, otherwise returns the offending rule so the
    /// caller can reject the request without touching balances.
    pub fn check_withdraw(
        &self,
        user_balance: i128,
        strategy_balance: i128,
        amount: i128,
    ) -> Result<(), MinimumBalanceViolation> {
        if amount <= 0 {
            return Err(MinimumBalanceViolation::NonPositiveAmount);
        }

        if amount > user_balance {
            return Err(MinimumBalanceViolation::InsufficientBalance);
        }

        let remaining_user = user_balance - amount;
        if remaining_user < self.user_minimum {
            return Err(MinimumBalanceViolation::BelowUserMinimum {
                remaining: remaining_user,
                minimum: self.user_minimum,
            });
        }

        let remaining_strategy = strategy_balance - amount;
        if remaining_strategy < self.strategy_minimum {
            return Err(MinimumBalanceViolation::BelowStrategyMinimum {
                remaining: remaining_strategy,
                minimum: self.strategy_minimum,
            });
        }

        Ok(())
    }
}

/// Reason a partial withdraw was rejected by the minimum balance policy.
#[derive(Clone, Debug, PartialEq)]
pub enum MinimumBalanceViolation {
    NonPositiveAmount,
    InsufficientBalance,
    BelowUserMinimum { remaining: i128, minimum: i128 },
    BelowStrategyMinimum { remaining: i128, minimum: i128 },
}

/// A time-based vesting plan for a single provider's incentive rewards.
///
/// `total_amount` is the full reward allocation. `start_time` is the ledger
/// timestamp at which vesting begins, `cliff` is the duration (in seconds)
/// that must elapse before any amount becomes releasable, and `duration` is
/// the total vesting window measured from `start_time`. `released` tracks the
/// amount already withdrawn so unvested balances stay inaccessible.
#[derive(Clone, Debug, PartialEq)]
pub struct VestingSchedule {
    pub total_amount: i128,
    pub start_time: u64,
    pub cliff: u64,
    pub duration: u64,
    pub released: i128,
}

impl VestingSchedule {
    pub fn new(total_amount: i128, start_time: u64, cliff: u64, duration: u64) -> Self {
        Self {
            total_amount,
            start_time,
            cliff,
            duration,
            released: 0,
        }
    }

    /// Deterministically compute the amount that has vested as of `now`.
    ///
    /// Before the cliff elapses nothing is vested. After the full duration
    /// the entire allocation is vested. In between, vesting is linear and
    /// integer-truncated so the result is fully deterministic.
    pub fn vested_amount(&self, now: u64) -> i128 {
        if now < self.start_time.saturating_add(self.cliff) {
            return 0;
        }

        let elapsed = now.saturating_sub(self.start_time);
        if self.duration == 0 || elapsed >= self.duration {
            return self.total_amount;
        }

        (self.total_amount * elapsed as i128) / self.duration as i128
    }

    /// Amount currently available to release: vested minus already released.
    /// Unvested balances remain inaccessible until scheduled release events.
    pub fn releasable_amount(&self, now: u64) -> i128 {
        (self.vested_amount(now) - self.released).max(0)
    }

    /// Release the currently vested amount, updating the released balance.
    /// Returns the amount released (0 when nothing is yet vested).
    pub fn release(&mut self, now: u64) -> i128 {
        let amount = self.releasable_amount(now);
        self.released += amount;
        amount
    }
}

pub fn get_rewards_pool_status(pool: &RewardsPool) -> RewardsPoolStatus {
    RewardsPoolStatus {
        balance: pool.balance,
        estimated_days_remaining: estimated_days_remaining(pool.balance, pool.daily_outflow),
        daily_outflow: pool.daily_outflow,
        auto_fund_threshold: pool.auto_fund_threshold,
    }
}

pub fn monitor_rewards_pool(pool: &mut RewardsPool) -> Option<RewardsPoolLow> {
    if pool.balance >= pool.auto_fund_threshold {
        return None;
    }

    let days_remaining = estimated_days_remaining(pool.balance, pool.daily_outflow);
    let fund_amount = DEFAULT_AUTO_FUND_AMOUNT.min(pool.treasury_balance);
    pool.balance += fund_amount;
    pool.treasury_balance -= fund_amount;

    Some(RewardsPoolLow {
        balance: pool.balance,
        days_remaining,
    })
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

    #[test]
    fn healthy_pool_status() {
        let mut pool = RewardsPool {
            balance: 10_000 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
        };

        assert_eq!(
            get_rewards_pool_status(&pool),
            RewardsPoolStatus {
                balance: 10_000 * XLM,
                estimated_days_remaining: 100,
                daily_outflow: 100 * XLM,
                auto_fund_threshold: 1_000 * XLM,
            }
        );
        assert_eq!(monitor_rewards_pool(&mut pool), None);
    }

    #[test]
    fn low_pool_auto_funds_from_treasury() {
        let mut pool = RewardsPool {
            balance: 500 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
        };

        let event = monitor_rewards_pool(&mut pool).unwrap();

        assert_eq!(event.days_remaining, 5);
        assert_eq!(pool.balance, 5_500 * XLM);
        assert_eq!(pool.treasury_balance, 15_000 * XLM);
    }

    #[test]
    fn empty_pool_reports_zero_days_and_funds_available_treasury() {
        let mut pool = RewardsPool {
            balance: 0,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 800 * XLM,
        };

        let event = monitor_rewards_pool(&mut pool).unwrap();

        assert_eq!(event.days_remaining, 0);
        assert_eq!(pool.balance, 800 * XLM);
        assert_eq!(pool.treasury_balance, 0);
    }

    #[test]
    fn nothing_vests_before_cliff() {
        let schedule = VestingSchedule::new(1_000 * XLM, 1_000, 500, 2_000);

        assert_eq!(schedule.vested_amount(1_000), 0);
        assert_eq!(schedule.vested_amount(1_499), 0);
        assert_eq!(schedule.releasable_amount(1_499), 0);
    }

    #[test]
    fn linear_vesting_is_deterministic() {
        let schedule = VestingSchedule::new(1_000 * XLM, 1_000, 0, 1_000);

        assert_eq!(schedule.vested_amount(1_250), 250 * XLM);
        assert_eq!(schedule.vested_amount(1_500), 500 * XLM);
        assert_eq!(schedule.vested_amount(2_000), 1_000 * XLM);
        assert_eq!(schedule.vested_amount(5_000), 1_000 * XLM);
    }

    #[test]
    fn release_only_unlocks_vested_amount() {
        let mut schedule = VestingSchedule::new(1_000 * XLM, 1_000, 0, 1_000);

        assert_eq!(schedule.release(1_250), 250 * XLM);
        assert_eq!(schedule.released, 250 * XLM);
        assert_eq!(schedule.releasable_amount(1_250), 0);

        assert_eq!(schedule.release(1_500), 250 * XLM);
        assert_eq!(schedule.released, 500 * XLM);

        assert_eq!(schedule.release(2_000), 500 * XLM);
        assert_eq!(schedule.released, 1_000 * XLM);
        assert_eq!(schedule.release(3_000), 0);
    }

    #[test]
    fn withdraw_within_minimums_is_allowed() {
        let policy = MinimumBalancePolicy::new(1_000 * XLM, 2_000 * XLM);

        assert_eq!(
            policy.check_withdraw(5_000 * XLM, 10_000 * XLM, 3_000 * XLM),
            Ok(())
        );
        // Withdrawing exactly down to the minimum is permitted.
        assert_eq!(
            policy.check_withdraw(5_000 * XLM, 10_000 * XLM, 4_000 * XLM),
            Ok(())
        );
    }

    #[test]
    fn withdraw_below_user_minimum_is_rejected() {
        let policy = MinimumBalancePolicy::new(1_000 * XLM, 0);

        assert_eq!(
            policy.check_withdraw(5_000 * XLM, 10_000 * XLM, 4_500 * XLM),
            Err(MinimumBalanceViolation::BelowUserMinimum {
                remaining: 500 * XLM,
                minimum: 1_000 * XLM,
            })
        );
    }

    #[test]
    fn withdraw_below_strategy_minimum_is_rejected() {
        let policy = MinimumBalancePolicy::new(0, 2_000 * XLM);

        assert_eq!(
            policy.check_withdraw(5_000 * XLM, 10_000 * XLM, 9_000 * XLM),
            Err(MinimumBalanceViolation::BelowStrategyMinimum {
                remaining: 1_000 * XLM,
                minimum: 2_000 * XLM,
            })
        );
    }

    #[test]
    fn invalid_withdraw_amounts_are_rejected() {
        let policy = MinimumBalancePolicy::new(1_000 * XLM, 1_000 * XLM);

        assert_eq!(
            policy.check_withdraw(5_000 * XLM, 10_000 * XLM, 0),
            Err(MinimumBalanceViolation::NonPositiveAmount)
        );
        assert_eq!(
            policy.check_withdraw(5_000 * XLM, 10_000 * XLM, 6_000 * XLM),
            Err(MinimumBalanceViolation::InsufficientBalance)
        );
    }
}
