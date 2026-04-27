//! Provider earnings report (Issue #366).
//!
//! Aggregates fee_shares_earned from per-day buckets written by `collect_fee`.
//! `stake_rewards_earned` and `subscription_fees_earned` are sourced from
//! StakeVault and UserPortfolio respectively; they are tracked as 0 here and
//! expected to be merged by an off-chain aggregator (or a future cross-contract
//! implementation).

use crate::storage::{
    get_provider_daily_fee_shares, get_provider_earnings_first_day,
};
use soroban_sdk::{contracttype, Address, Env};
use stellar_swipe_common::SECONDS_PER_DAY;

// ── Period enum ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportPeriod {
    Daily,
    Weekly,
    Monthly,
    AllTime,
}

// ── Report struct ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EarningsReport {
    pub fee_shares_earned: i128,
    /// Stake rewards from StakeVault (0 — cross-contract aggregation required).
    pub stake_rewards_earned: i128,
    /// Subscription fees from UserPortfolio (0 — cross-contract aggregation required).
    pub subscription_fees_earned: i128,
    pub total_earned: i128,
    pub period_start: u64,
    pub period_end: u64,
}

// ── Core logic ────────────────────────────────────────────────────────────────

fn current_day(env: &Env) -> u64 {
    env.ledger().timestamp() / SECONDS_PER_DAY
}

/// Sum `fee_shares` from `start_day..=end_day` (inclusive) for the given provider.
fn sum_fee_shares_days(env: &Env, provider: &Address, start_day: u64, end_day: u64) -> i128 {
    let mut total: i128 = 0;
    let mut day = start_day;
    while day <= end_day {
        total = total.saturating_add(get_provider_daily_fee_shares(env, provider, day));
        day = day.saturating_add(1);
    }
    total
}

/// Returns an `EarningsReport` for `provider` over `period`.
///
/// | Period  | Window                        |
/// |---------|-------------------------------|
/// | Daily   | last 1 day                    |
/// | Weekly  | last 7 days                   |
/// | Monthly | last 30 days                  |
/// | AllTime | from first recorded earnings  |
pub fn get_provider_earnings_report(
    env: &Env,
    provider: &Address,
    period: ReportPeriod,
) -> EarningsReport {
    let today = current_day(env);
    let now_ts = env.ledger().timestamp();

    let (start_day, start_ts) = match period {
        ReportPeriod::Daily => {
            let d = today.saturating_sub(1);
            (d, d * SECONDS_PER_DAY)
        }
        ReportPeriod::Weekly => {
            let d = today.saturating_sub(7);
            (d, d * SECONDS_PER_DAY)
        }
        ReportPeriod::Monthly => {
            let d = today.saturating_sub(30);
            (d, d * SECONDS_PER_DAY)
        }
        ReportPeriod::AllTime => {
            let first = get_provider_earnings_first_day(env, provider).unwrap_or(today);
            (first, first * SECONDS_PER_DAY)
        }
    };

    let fee_shares = sum_fee_shares_days(env, provider, start_day, today);

    EarningsReport {
        fee_shares_earned: fee_shares,
        stake_rewards_earned: 0,
        subscription_fees_earned: 0,
        total_earned: fee_shares,
        period_start: start_ts,
        period_end: now_ts,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::add_provider_daily_fee_shares;
    use soroban_sdk::{
        contract, contractimpl,
        testutils::{Address as _, Ledger},
        Env,
    };

    #[contract]
    struct MockContract;

    #[contractimpl]
    impl MockContract {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        let id = env.register(MockContract, ());
        (env, id)
    }

    /// Simulate 30 days of earnings (10 units/day) and verify monthly report.
    #[test]
    fn test_monthly_report_sums_30_days() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);

        env.as_contract(&contract_id, || {
            // Seed 30 days of fee share data
            let base_day: u64 = 100; // arbitrary starting day
            for i in 0u64..30 {
                add_provider_daily_fee_shares(&env, &provider, base_day + i, 10);
            }

            // Set ledger timestamp to day 130 (just past the 30-day window)
            env.ledger().with_mut(|l| {
                l.timestamp = (base_day + 30) * SECONDS_PER_DAY;
            });

            let report = get_provider_earnings_report(&env, &provider, ReportPeriod::Monthly);
            assert_eq!(report.fee_shares_earned, 300); // 30 days × 10 each
            assert_eq!(report.stake_rewards_earned, 0);
            assert_eq!(report.subscription_fees_earned, 0);
            assert_eq!(report.total_earned, 300);
        });
    }

    #[test]
    fn test_daily_report_covers_last_1_day() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let today: u64 = 200;
            env.ledger().with_mut(|l| {
                l.timestamp = today * SECONDS_PER_DAY;
            });

            add_provider_daily_fee_shares(&env, &provider, today - 1, 50);
            add_provider_daily_fee_shares(&env, &provider, today - 5, 999); // outside window

            let report = get_provider_earnings_report(&env, &provider, ReportPeriod::Daily);
            assert_eq!(report.fee_shares_earned, 50);
        });
    }

    #[test]
    fn test_weekly_report_covers_last_7_days() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let today: u64 = 200;
            env.ledger().with_mut(|l| {
                l.timestamp = today * SECONDS_PER_DAY;
            });

            for i in 0u64..7 {
                add_provider_daily_fee_shares(&env, &provider, today - 7 + i, 20);
            }
            // Day outside window
            add_provider_daily_fee_shares(&env, &provider, today - 8, 999);

            let report = get_provider_earnings_report(&env, &provider, ReportPeriod::Weekly);
            assert_eq!(report.fee_shares_earned, 140); // 7 days × 20
        });
    }

    #[test]
    fn test_all_time_report_covers_full_history() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let today: u64 = 300;
            env.ledger().with_mut(|l| {
                l.timestamp = today * SECONDS_PER_DAY;
            });

            // Record earnings spread across 200 days
            add_provider_daily_fee_shares(&env, &provider, 100, 100);
            add_provider_daily_fee_shares(&env, &provider, 200, 200);
            add_provider_daily_fee_shares(&env, &provider, 300, 50);

            let report = get_provider_earnings_report(&env, &provider, ReportPeriod::AllTime);
            assert_eq!(report.fee_shares_earned, 350);
            // period_start == first earnings day × SECONDS_PER_DAY
            assert_eq!(report.period_start, 100 * SECONDS_PER_DAY);
        });
    }

    #[test]
    fn test_no_earnings_returns_zeros() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);

        env.as_contract(&contract_id, || {
            env.ledger().with_mut(|l| {
                l.timestamp = 1_000_000;
            });
            let report = get_provider_earnings_report(&env, &provider, ReportPeriod::Monthly);
            assert_eq!(report.fee_shares_earned, 0);
            assert_eq!(report.total_earned, 0);
        });
    }
}
