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
}
