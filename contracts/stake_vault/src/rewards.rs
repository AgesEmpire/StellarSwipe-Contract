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
}
