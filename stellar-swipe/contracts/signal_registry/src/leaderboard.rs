//! Pre-aggregated provider leaderboard with four sort metrics.
//!
//! Four sorted index arrays (one per metric) are maintained in persistent storage,
//! each capped at INDEX_CAPACITY. Updated on every signal close via
//! update_leaderboard_index. Queries are O(1) storage reads.
//!
//! Qualification: provider must have >= MIN_CLOSED_SIGNALS (10) closed signals.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

use crate::stake;
use crate::types::ProviderPerformance;

pub const MIN_CLOSED_SIGNALS: u32 = 10;
pub const DEFAULT_LEADERBOARD_LIMIT: u32 = 10;
pub const MAX_LEADERBOARD_LIMIT: u32 = 50;
pub const INDEX_CAPACITY: u32 = 100;

// ── Public types ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderMetric {
    BySuccessRate,
    ByTotalAdopters,
    ByTotalProfitDelta,
    ByStake,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ProviderLeaderboardEntry {
    pub rank: u32,
    pub provider: Address,
    pub metric_value: i128,
    pub total_signals: u32,
    pub verified: bool,
}

// ── Legacy aliases ────────────────────────────────────────────────────────────

pub type ProviderLeaderboard = ProviderLeaderboardEntry;

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaderboardMetric {
    SuccessRate,
    Volume,
    Followers,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum LeaderboardKey {
    SuccessRateIndex,
    AdoptersIndex,
    ProfitDeltaIndex,
    StakeIndex,
}

// ── Index entry ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct IndexEntry {
    pub provider: Address,
    pub closed_signals: u32,
    pub success_rate: u32,
    pub total_adopters: u32,
    pub total_profit_delta: i128,
    pub stake_amount: i128,
    pub verified: bool,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn load_index(env: &Env, key: LeaderboardKey) -> Vec<IndexEntry> {
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

fn save_index(env: &Env, key: LeaderboardKey, index: &Vec<IndexEntry>) {
    env.storage().persistent().set(&key, index);
}

fn is_qualified(entry: &IndexEntry) -> bool {
    entry.closed_signals >= MIN_CLOSED_SIGNALS
}

fn upsert_sorted<F>(env: &Env, index: &mut Vec<IndexEntry>, entry: IndexEntry, score_fn: F)
where
    F: Fn(&IndexEntry) -> i128,
{
    let mut without: Vec<IndexEntry> = Vec::new(env);
    for i in 0..index.len() {
        let e = index.get(i).unwrap();
        if e.provider != entry.provider {
            without.push_back(e);
        }
    }

    if !is_qualified(&entry) {
        *index = without;
        return;
    }

    let entry_score = score_fn(&entry);
    let mut insert_at = without.len();
    for i in 0..without.len() {
        if score_fn(&without.get(i).unwrap()) < entry_score {
            insert_at = i;
            break;
        }
    }

    let mut result: Vec<IndexEntry> = Vec::new(env);
    for i in 0..insert_at {
        result.push_back(without.get(i).unwrap());
    }
    result.push_back(entry);
    for i in insert_at..without.len() {
        result.push_back(without.get(i).unwrap());
    }

    let cap = INDEX_CAPACITY.min(result.len());
    let mut capped: Vec<IndexEntry> = Vec::new(env);
    for i in 0..cap {
        capped.push_back(result.get(i).unwrap());
    }
    *index = capped;
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn update_leaderboard_index(env: &Env, provider: Address, stats: &ProviderPerformance) {
    let stake_info = stake::get_stake_info(env, &provider);
    let stake_amount = stake_info.as_ref().map(|s| s.amount).unwrap_or(0);
    let verified = stake_amount >= stake::DEFAULT_MINIMUM_STAKE;

    let closed_signals = stats
        .successful_signals
        .saturating_add(stats.failed_signals);

    let entry = IndexEntry {
        provider: provider.clone(),
        closed_signals,
        success_rate: stats.success_rate,
        total_adopters: stats.total_copies as u32,
        total_profit_delta: stats.avg_return.saturating_mul(closed_signals as i128),
        stake_amount,
        verified,
    };

    let mut sr = load_index(env, LeaderboardKey::SuccessRateIndex);
    upsert_sorted(env, &mut sr, entry.clone(), |e| e.success_rate as i128);
    save_index(env, LeaderboardKey::SuccessRateIndex, &sr);

    let mut ad = load_index(env, LeaderboardKey::AdoptersIndex);
    upsert_sorted(env, &mut ad, entry.clone(), |e| e.total_adopters as i128);
    save_index(env, LeaderboardKey::AdoptersIndex, &ad);

    let mut pd = load_index(env, LeaderboardKey::ProfitDeltaIndex);
    upsert_sorted(env, &mut pd, entry.clone(), |e| e.total_profit_delta);
    save_index(env, LeaderboardKey::ProfitDeltaIndex, &pd);

    let mut sk = load_index(env, LeaderboardKey::StakeIndex);
    upsert_sorted(env, &mut sk, entry, |e| e.stake_amount);
    save_index(env, LeaderboardKey::StakeIndex, &sk);

    env.events()
        .publish((symbol_short!("lb_upd"), provider), stats.success_rate);
}

pub fn get_provider_leaderboard(
    env: &Env,
    metric: ProviderMetric,
    limit: u32,
) -> Vec<ProviderLeaderboardEntry> {
    let limit = if limit == 0 {
        DEFAULT_LEADERBOARD_LIMIT
    } else {
        limit.min(MAX_LEADERBOARD_LIMIT)
    };

    let key = match metric {
        ProviderMetric::BySuccessRate => LeaderboardKey::SuccessRateIndex,
        ProviderMetric::ByTotalAdopters => LeaderboardKey::AdoptersIndex,
        ProviderMetric::ByTotalProfitDelta => LeaderboardKey::ProfitDeltaIndex,
        ProviderMetric::ByStake => LeaderboardKey::StakeIndex,
    };

    let index = load_index(env, key);
    let take = limit.min(index.len());
    let mut result = Vec::new(env);

    for i in 0..take {
        let e = index.get(i).unwrap();
        let metric_value = match metric {
            ProviderMetric::BySuccessRate => e.success_rate as i128,
            ProviderMetric::ByTotalAdopters => e.total_adopters as i128,
            ProviderMetric::ByTotalProfitDelta => e.total_profit_delta,
            ProviderMetric::ByStake => e.stake_amount,
        };
        result.push_back(ProviderLeaderboardEntry {
            rank: i + 1,
            provider: e.provider,
            metric_value,
            total_signals: e.closed_signals,
            verified: e.verified,
        });
    }

    result
}

/// Legacy wrapper kept for backward-compat with existing get_leaderboard callers.
pub fn get_leaderboard(
    env: &Env,
    _stats_map: &soroban_sdk::Map<Address, ProviderPerformance>,
    metric: LeaderboardMetric,
    limit: u32,
) -> Vec<ProviderLeaderboardEntry> {
    let pm = match metric {
        LeaderboardMetric::SuccessRate => ProviderMetric::BySuccessRate,
        LeaderboardMetric::Volume => ProviderMetric::ByTotalProfitDelta,
        LeaderboardMetric::Followers => return Vec::new(env),
    };
    get_provider_leaderboard(env, pm, limit)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProviderPerformance;
    use soroban_sdk::testutils::Address as TestAddress;
    use soroban_sdk::{contract, Env};

    #[contract]
    struct TestContract;

    fn make_stats(
        success_rate: u32,
        total_copies: u64,
        avg_return: i128,
        successful: u32,
        failed: u32,
    ) -> ProviderPerformance {
        ProviderPerformance {
            total_signals: successful + failed,
            successful_signals: successful,
            failed_signals: failed,
            total_copies,
            success_rate,
            avg_return,
            total_volume: 0,
        }
    }

    /// 30 providers with varied metrics — verify top-10 by each metric.
    #[test]
    fn test_30_providers_top_10_by_each_metric() {
        let env = Env::default();
        let cid = env.register(TestContract, ());

        env.as_contract(&cid, || {
            // Provider i:
            //   success_rate   = (i+1)*100   bps  (100..=3000)
            //   total_copies   = (i+1)*5          (5..=150)
            //   avg_return     = (i as i128-14)*10 (-140..=150)
            //   closed_signals = 10+i              (10..=39, all qualify)
            for i in 0..30u32 {
                let p = Address::generate(&env);
                let closed = 10 + i;
                let stats = make_stats(
                    (i + 1) * 100,
                    ((i + 1) * 5) as u64,
                    (i as i128 - 14) * 10,
                    closed / 2 + 1,
                    closed / 2,
                );
                update_leaderboard_index(&env, p, &stats);
            }

            // BY_SUCCESS_RATE
            let lb = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            assert_eq!(lb.len(), 10);
            assert_eq!(lb.get(0).unwrap().metric_value, 3000);
            assert_eq!(lb.get(0).unwrap().rank, 1);
            for i in 0..9u32 {
                assert!(
                    lb.get(i).unwrap().metric_value >= lb.get(i + 1).unwrap().metric_value
                );
            }

            // BY_TOTAL_ADOPTERS
            let lb = get_provider_leaderboard(&env, ProviderMetric::ByTotalAdopters, 10);
            assert_eq!(lb.len(), 10);
            assert_eq!(lb.get(0).unwrap().metric_value, 150);
            for i in 0..9u32 {
                assert!(
                    lb.get(i).unwrap().metric_value >= lb.get(i + 1).unwrap().metric_value
                );
            }

            // BY_TOTAL_PROFIT_DELTA
            let lb = get_provider_leaderboard(&env, ProviderMetric::ByTotalProfitDelta, 10);
            assert_eq!(lb.len(), 10);
            for i in 0..9u32 {
                assert!(
                    lb.get(i).unwrap().metric_value >= lb.get(i + 1).unwrap().metric_value
                );
            }

            // BY_STAKE — no stakes set, all zero; verify <= 10 and descending
            let lb_stake = get_provider_leaderboard(&env, ProviderMetric::ByStake, 10);
            let n = lb_stake.len();
            assert!(n <= 10);
            for i in 0..n.saturating_sub(1) {
                assert!(
                    lb_stake.get(i).unwrap().metric_value
                        >= lb_stake.get(i + 1).unwrap().metric_value
                );
            }
        });
    }

    #[test]
    fn test_under_min_signals_excluded() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            // 9 closed signals — below threshold
            let stats = make_stats(8000, 50, 100, 5, 4);
            update_leaderboard_index(&env, p, &stats);
            let lb = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            assert_eq!(lb.len(), 0);
        });
    }

    #[test]
    fn test_exactly_min_signals_qualifies() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            let stats = make_stats(7000, 20, 50, 5, 5); // 10 closed
            update_leaderboard_index(&env, p, &stats);
            let lb = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            assert_eq!(lb.len(), 1);
            assert_eq!(lb.get(0).unwrap().total_signals, 10);
        });
    }

    #[test]
    fn test_upsert_no_duplicates() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            update_leaderboard_index(&env, p.clone(), &make_stats(5000, 10, 50, 6, 5));
            update_leaderboard_index(&env, p.clone(), &make_stats(9000, 30, 200, 8, 5));
            let lb = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            assert_eq!(lb.len(), 1);
            assert_eq!(lb.get(0).unwrap().metric_value, 9000);
        });
    }

    #[test]
    fn test_verified_flag_without_stake() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            update_leaderboard_index(&env, p, &make_stats(8000, 20, 100, 6, 5));
            let lb = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            assert_eq!(lb.len(), 1);
            assert!(!lb.get(0).unwrap().verified);
        });
    }

    #[test]
    fn test_legacy_get_leaderboard_wrapper() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            update_leaderboard_index(&env, p, &make_stats(7500, 15, 80, 6, 5));
            let empty_map = soroban_sdk::Map::new(&env);
            let lb = get_leaderboard(&env, &empty_map, LeaderboardMetric::SuccessRate, 10);
            assert_eq!(lb.len(), 1);
            let lb_f = get_leaderboard(&env, &empty_map, LeaderboardMetric::Followers, 10);
            assert_eq!(lb_f.len(), 0);
        });
    }
}
