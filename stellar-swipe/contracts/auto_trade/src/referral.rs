#![allow(dead_code)]
use soroban_sdk::{contracttype, Address, Env, Map, Symbol};

use crate::errors::AutoTradeError;

// ── Constants ────────────────────────────────────────────────────────────────

/// 90 days in seconds
pub const REFERRAL_WINDOW_SECS: u64 = 90 * 24 * 60 * 60;
/// 10% of the platform fee goes to the referrer
pub const REFERRAL_REWARD_BPS: i128 = 10; // 10 %
/// Max active referrals per user
pub const MAX_ACTIVE_REFERRALS: u32 = 100;
/// Max trades within which referral reward is active
pub const MAX_REFERRAL_TRADES: u32 = 100;

// ── Storage types ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferralStats {
    pub total_referrals: u32,
    pub active_referrals: u32,
    pub total_earnings: i128,
    /// Per-asset earnings: asset_id → earned amount
    pub earnings_by_asset: Map<u32, i128>,
}

/// Stored per referee: who referred them and when they signed up.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferralEntry {
    pub referrer: Address,
    pub signup_ts: u64,
    pub trade_count: u32,
}

#[contracttype]
pub enum ReferralKey {
    /// referee → ReferralEntry
    Entry(Address),
    /// referrer → ReferralStats
    Stats(Address),
    /// referrer → Vec<Address> (active referee list for cycle detection)
    Referees(Address),
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn get_entry(env: &Env, referee: &Address) -> Option<ReferralEntry> {
    env.storage()
        .persistent()
        .get(&ReferralKey::Entry(referee.clone()))
}

fn set_entry(env: &Env, referee: &Address, entry: &ReferralEntry) {
    env.storage()
        .persistent()
        .set(&ReferralKey::Entry(referee.clone()), entry);
}

pub fn get_stats(env: &Env, referrer: &Address) -> ReferralStats {
    env.storage()
        .persistent()
        .get(&ReferralKey::Stats(referrer.clone()))
        .unwrap_or_else(|| ReferralStats {
            total_referrals: 0,
            active_referrals: 0,
            total_earnings: 0,
            earnings_by_asset: Map::new(env),
        })
}

fn set_stats(env: &Env, referrer: &Address, stats: &ReferralStats) {
    env.storage()
        .persistent()
        .set(&ReferralKey::Stats(referrer.clone()), stats);
}

/// Detect cycles: walk up the referrer chain and check if `candidate` appears.
fn has_cycle(env: &Env, start: &Address, candidate: &Address) -> bool {
    let mut current = start.clone();
    // Max depth = MAX_ACTIVE_REFERRALS to bound gas
    for _ in 0..MAX_ACTIVE_REFERRALS {
        if let Some(entry) = get_entry(env, &current) {
            if &entry.referrer == candidate {
                return true;
            }
            current = entry.referrer;
        } else {
            break;
        }
    }
    false
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Register `referrer` as the one who referred `referee`.
/// Must be called before the referee executes any trade.
pub fn set_referrer(
    env: &Env,
    referee: &Address,
    referrer: &Address,
) -> Result<(), AutoTradeError> {
    // Cannot self-refer
    if referee == referrer {
        return Err(AutoTradeError::SelfReferral);
    }

    // Block if already set
    if get_entry(env, referee).is_some() {
        return Err(AutoTradeError::ReferralAlreadySet);
    }

    // Detect circular referral (A→B, B→C, C→A)
    if has_cycle(env, referrer, referee) {
        return Err(AutoTradeError::CircularReferral);
    }

    // Enforce max active referrals cap on referrer
    let mut stats = get_stats(env, referrer);
    if stats.active_referrals >= MAX_ACTIVE_REFERRALS {
        return Err(AutoTradeError::ReferralLimitExceeded);
    }

    let entry = ReferralEntry {
        referrer: referrer.clone(),
        signup_ts: env.ledger().timestamp(),
        trade_count: 0,
    };
    set_entry(env, referee, &entry);

    stats.total_referrals += 1;
    stats.active_referrals += 1;
    set_stats(env, referrer, &stats);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "referral_registered"),
            referrer.clone(),
            referee.clone(),
        ),
        env.ledger().timestamp(),
    );

    Ok(())
}

/// Called during trade execution.  
/// Returns the referral reward amount (10 % of `platform_fee`) if active,
/// updates stats, emits event, and increments the trade counter.
/// Returns 0 if no active referral applies.
pub fn process_referral_reward(
    env: &Env,
    referee: &Address,
    asset_id: u32,
    platform_fee: i128,
) -> i128 {
    let mut entry = match get_entry(env, referee) {
        Some(e) => e,
        None => return 0,
    };

    let now = env.ledger().timestamp();
    let elapsed = now.saturating_sub(entry.signup_ts);

    // Check expiry conditions
    let expired = elapsed > REFERRAL_WINDOW_SECS || entry.trade_count >= MAX_REFERRAL_TRADES;
    if expired {
        // Deactivate: decrement active count on referrer
        let mut stats = get_stats(env, &entry.referrer);
        if stats.active_referrals > 0 {
            stats.active_referrals -= 1;
        }
        set_stats(env, &entry.referrer, &stats);
        // Remove entry so future calls skip immediately
        env.storage()
            .persistent()
            .remove(&ReferralKey::Entry(referee.clone()));
        return 0;
    }

    let reward = platform_fee * REFERRAL_REWARD_BPS / 100;
    if reward <= 0 {
        return 0;
    }

    // Update trade counter
    entry.trade_count += 1;
    set_entry(env, referee, &entry);

    // Update referrer stats
    let mut stats = get_stats(env, &entry.referrer);
    stats.total_earnings += reward;
    let prev = stats.earnings_by_asset.get(asset_id).unwrap_or(0);
    stats.earnings_by_asset.set(asset_id, prev + reward);

    // If this trade hit the trade cap, deactivate
    if entry.trade_count >= MAX_REFERRAL_TRADES {
        if stats.active_referrals > 0 {
            stats.active_referrals -= 1;
        }
    }
    set_stats(env, &entry.referrer, &stats);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "referral_reward_earned"),
            entry.referrer.clone(),
            referee.clone(),
        ),
        (asset_id, reward),
    );

    reward
}

/// Query referral entry for a referee (returns None if not referred / expired).
pub fn get_referral_entry(env: &Env, referee: &Address) -> Option<ReferralEntry> {
    get_entry(env, referee)
}

/// Query referral stats for a referrer.
pub fn get_referral_stats(env: &Env, referrer: &Address) -> ReferralStats {
    get_stats(env, referrer)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let addr = env.register(TestContract, ());
        (env, addr)
    }

    // ── set_referrer ──────────────────────────────────────────────────────────

    #[test]
    fn test_set_referrer_success() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            set_referrer(&env, &referee, &referrer).unwrap();
            let entry = get_entry(&env, &referee).unwrap();
            assert_eq!(entry.referrer, referrer);
            assert_eq!(entry.trade_count, 0);

            let stats = get_stats(&env, &referrer);
            assert_eq!(stats.total_referrals, 1);
            assert_eq!(stats.active_referrals, 1);
        });
    }

    #[test]
    fn test_self_referral_blocked() {
        let (env, contract_addr) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            let err = set_referrer(&env, &user, &user).unwrap_err();
            assert_eq!(err, AutoTradeError::SelfReferral);
        });
    }

    #[test]
    fn test_referral_already_set_blocked() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            set_referrer(&env, &referee, &referrer).unwrap();
            let err = set_referrer(&env, &referee, &referrer).unwrap_err();
            assert_eq!(err, AutoTradeError::ReferralAlreadySet);
        });
    }

    #[test]
    fn test_circular_referral_blocked() {
        let (env, contract_addr) = setup();
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let c = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            // A refers B, B refers C — now C trying to refer A would be circular
            set_referrer(&env, &b, &a).unwrap();
            set_referrer(&env, &c, &b).unwrap();
            let err = set_referrer(&env, &a, &c).unwrap_err();
            assert_eq!(err, AutoTradeError::CircularReferral);
        });
    }

    #[test]
    fn test_max_referral_limit_enforced() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            // Manually set active_referrals to MAX
            let stats = ReferralStats {
                total_referrals: MAX_ACTIVE_REFERRALS,
                active_referrals: MAX_ACTIVE_REFERRALS,
                total_earnings: 0,
                earnings_by_asset: Map::new(&env),
            };
            set_stats(&env, &referrer, &stats);

            let new_referee = Address::generate(&env);
            let err = set_referrer(&env, &new_referee, &referrer).unwrap_err();
            assert_eq!(err, AutoTradeError::ReferralLimitExceeded);
        });
    }

    // ── process_referral_reward ───────────────────────────────────────────────

    #[test]
    fn test_reward_10_percent_of_platform_fee() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            set_referrer(&env, &referee, &referrer).unwrap();

            // platform_fee = 10 XLM (in stroops: 10_000_000)
            let reward = process_referral_reward(&env, &referee, 1, 10_000_000);
            assert_eq!(reward, 1_000_000); // 10%

            let stats = get_stats(&env, &referrer);
            assert_eq!(stats.total_earnings, 1_000_000);
            assert_eq!(stats.earnings_by_asset.get(1u32).unwrap(), 1_000_000);
        });
    }

    #[test]
    fn test_reward_accumulates_over_multiple_trades() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            set_referrer(&env, &referee, &referrer).unwrap();

            for _ in 0..10 {
                process_referral_reward(&env, &referee, 1, 10_000_000);
            }

            let stats = get_stats(&env, &referrer);
            assert_eq!(stats.total_earnings, 10 * 1_000_000);
        });
    }

    #[test]
    fn test_reward_stops_after_90_days() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            set_referrer(&env, &referee, &referrer).unwrap();

            // Advance time past 90 days
            env.ledger()
                .set_timestamp(1_000_000 + REFERRAL_WINDOW_SECS + 1);

            let reward = process_referral_reward(&env, &referee, 1, 10_000_000);
            assert_eq!(reward, 0);

            // active_referrals should be decremented
            let stats = get_stats(&env, &referrer);
            assert_eq!(stats.active_referrals, 0);
        });
    }

    #[test]
    fn test_reward_stops_after_100_trades() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            set_referrer(&env, &referee, &referrer).unwrap();

            // Execute exactly MAX_REFERRAL_TRADES trades
            for _ in 0..MAX_REFERRAL_TRADES {
                process_referral_reward(&env, &referee, 1, 10_000_000);
            }

            // 101st trade should yield 0
            let reward = process_referral_reward(&env, &referee, 1, 10_000_000);
            assert_eq!(reward, 0);
        });
    }

    #[test]
    fn test_no_reward_without_referral() {
        let (env, contract_addr) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            let reward = process_referral_reward(&env, &user, 1, 10_000_000);
            assert_eq!(reward, 0);
        });
    }

    #[test]
    fn test_101st_referral_blocked() {
        let (env, contract_addr) = setup();
        let referrer = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            // Fill up to exactly MAX
            let stats = ReferralStats {
                total_referrals: MAX_ACTIVE_REFERRALS,
                active_referrals: MAX_ACTIVE_REFERRALS,
                total_earnings: 0,
                earnings_by_asset: Map::new(&env),
            };
            set_stats(&env, &referrer, &stats);

            let new_referee = Address::generate(&env);
            let err = set_referrer(&env, &new_referee, &referrer).unwrap_err();
            assert_eq!(err, AutoTradeError::ReferralLimitExceeded);
        });
    }

    #[test]
    fn test_get_referral_stats_empty() {
        let (env, contract_addr) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            let stats = get_referral_stats(&env, &user);
            assert_eq!(stats.total_referrals, 0);
            assert_eq!(stats.active_referrals, 0);
            assert_eq!(stats.total_earnings, 0);
        });
    }

    #[test]
    fn test_get_referral_entry_none_for_unreferred() {
        let (env, contract_addr) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_addr, || {
            assert!(get_referral_entry(&env, &user).is_none());
        });
    }
}
