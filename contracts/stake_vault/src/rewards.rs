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

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderCapConfig {
    pub max_provider_stake: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderCapExceeded {
    pub attempted: i128,
    pub cap: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StakeAllocated {
    pub amount: i128,
    pub total_stake: i128,
}

pub fn set_provider_cap(max_provider_stake: i128) -> Result<ProviderCapConfig, &'static str> {
    if max_provider_stake <= 0 {
        return Err("provider cap must be positive");
    }

    Ok(ProviderCapConfig {
        max_provider_stake,
    })
}

pub fn allocate_stake(
    config: &ProviderCapConfig,
    current_stake: i128,
    amount: i128,
) -> Result<StakeAllocated, ProviderCapExceeded> {
    let attempted = current_stake + amount;

    if attempted > config.max_provider_stake {
        return Err(ProviderCapExceeded {
            attempted,
            cap: config.max_provider_stake,
        });
    }

    Ok(StakeAllocated {
        amount,
        total_stake: attempted,
    })
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
    fn provider_cap_rejects_non_positive_values() {
        assert_eq!(set_provider_cap(0), Err("provider cap must be positive"));
        assert_eq!(set_provider_cap(-1), Err("provider cap must be positive"));
        assert_eq!(
            set_provider_cap(1_000 * XLM),
            Ok(ProviderCapConfig {
                max_provider_stake: 1_000 * XLM,
            })
        );
    }

    #[test]
    fn allocation_over_cap_is_rejected_before_mutation() {
        let config = set_provider_cap(1_000 * XLM).unwrap();
        let current_stake = 900 * XLM;

        let result = allocate_stake(&config, current_stake, 200 * XLM);

        assert_eq!(
            result,
            Err(ProviderCapExceeded {
                attempted: 1_100 * XLM,
                cap: 1_000 * XLM,
            })
        );
        assert_eq!(current_stake, 900 * XLM);
    }

    #[test]
    fn allocation_within_cap_records_accepted_values() {
        let config = set_provider_cap(1_000 * XLM).unwrap();

        let result = allocate_stake(&config, 900 * XLM, 100 * XLM).unwrap();

        assert_eq!(
            result,
            StakeAllocated {
                amount: 100 * XLM,
                total_stake: 1_000 * XLM,
            }
        );
    }
}
