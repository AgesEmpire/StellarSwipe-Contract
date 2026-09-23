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

/// Tracks the last applied deposit so repeated attempts for the same
/// transaction cannot duplicate balances or rewards.
#[derive(Clone, Debug, PartialEq)]
pub struct DepositRetryGuard {
    pub last_deposit_id: u64,
    pub last_deposit_amount: i128,
    pub applied: bool,
}

impl DepositRetryGuard {
    pub fn new() -> Self {
        DepositRetryGuard {
            last_deposit_id: 0,
            last_deposit_amount: 0,
            applied: false,
        }
    }
}

impl Default for DepositRetryGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of attempting to apply a deposit against the retry guard.
#[derive(Clone, Debug, PartialEq)]
pub enum DepositOutcome {
    /// Deposit was applied for the first time.
    Applied { deposit_id: u64, amount: i128 },
    /// Deposit was already applied; no state change was made.
    AlreadyApplied { deposit_id: u64 },
    /// Deposit was rejected (invalid amount); state left untouched.
    Rejected { deposit_id: u64 },
}

/// Applies a deposit to the rewards pool exactly once per `deposit_id`.
///
/// Repeated attempts with the same `deposit_id` are detected via the
/// `DepositRetryGuard` marker and skipped, so balances and rewards are never
/// duplicated. Invalid amounts are rejected without mutating state, leaving
/// the contract consistent for a later retry.
pub fn apply_deposit(
    pool: &mut RewardsPool,
    guard: &mut DepositRetryGuard,
    deposit_id: u64,
    amount: i128,
) -> DepositOutcome {
    if amount <= 0 {
        return DepositOutcome::Rejected { deposit_id };
    }

    if guard.applied && guard.last_deposit_id == deposit_id {
        return DepositOutcome::AlreadyApplied { deposit_id };
    }

    pool.balance += amount;
    guard.last_deposit_id = deposit_id;
    guard.last_deposit_amount = amount;
    guard.applied = true;

    DepositOutcome::Applied { deposit_id, amount }
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

    fn sample_pool() -> RewardsPool {
        RewardsPool {
            balance: 10_000 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
        }
    }

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
    fn deposit_applies_once() {
        let mut pool = sample_pool();
        let mut guard = DepositRetryGuard::new();

        let outcome = apply_deposit(&mut pool, &mut guard, 1, 250 * XLM);

        assert_eq!(
            outcome,
            DepositOutcome::Applied {
                deposit_id: 1,
                amount: 250 * XLM,
            }
        );
        assert_eq!(pool.balance, 10_250 * XLM);
    }

    #[test]
    fn repeated_deposit_is_not_duplicated() {
        let mut pool = sample_pool();
        let mut guard = DepositRetryGuard::new();

        apply_deposit(&mut pool, &mut guard, 7, 100 * XLM);
        let retry = apply_deposit(&mut pool, &mut guard, 7, 100 * XLM);

        assert_eq!(retry, DepositOutcome::AlreadyApplied { deposit_id: 7 });
        assert_eq!(pool.balance, 10_100 * XLM);
    }

    #[test]
    fn rejected_deposit_leaves_state_consistent() {
        let mut pool = sample_pool();
        let mut guard = DepositRetryGuard::new();

        let outcome = apply_deposit(&mut pool, &mut guard, 3, 0);

        assert_eq!(outcome, DepositOutcome::Rejected { deposit_id: 3 });
        assert_eq!(pool.balance, 10_000 * XLM);
        assert!(!guard.applied);

        // A valid retry after rejection still applies cleanly.
        let retry = apply_deposit(&mut pool, &mut guard, 3, 50 * XLM);
        assert_eq!(
            retry,
            DepositOutcome::Applied {
                deposit_id: 3,
                amount: 50 * XLM,
            }
        );
        assert_eq!(pool.balance, 10_050 * XLM);
    }
}
