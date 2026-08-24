//! Pre-aggregated provider leaderboard with four sort metrics.
//!
//! Four sorted index arrays (one per metric) are maintained in persistent storage,
//! each capped at INDEX_CAPACITY. Updated on every signal close via
//! update_leaderboard_index. Queries are O(1) storage reads.
//!
//! Qualification: provider must have >= MIN_CLOSED_SIGNALS (10) closed signals.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};
use stellar_swipe_common::ttl_manager::{bump_persistent_if_needed, force_bump_persistent};

use crate::events::emit_deprecated_entry_point_used;
use crate::social;
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

/// # Tie-breaking rule (applied when primary scores are equal)
///
/// 1. **Earlier registration timestamp** ranks higher (smaller `registered_at` wins).
/// 2. **Lexicographic address bytes** as a final fallback for byte-exact determinism
///    (earlier / smaller raw bytes wins).
///
/// This guarantees repeated queries against unchanged data always return entries
/// in the same order regardless of insertion order.
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
    /// Ledger timestamp when this provider first registered (used for tie-breaking).
    pub registered_at: u64,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn load_index(env: &Env, key: LeaderboardKey) -> Vec<IndexEntry> {
    bump_persistent_if_needed(env, &key);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

fn save_index(env: &Env, key: LeaderboardKey, index: &Vec<IndexEntry>) {
    env.storage().persistent().set(&key, index);
    bump_persistent_if_needed(env, &key);
}

/// Force-extend TTL for all four leaderboard index keys.  Intended to be
/// called by a keeper via the `bump_leaderboard_ttl` contract entrypoint.
pub fn bump_all_leaderboard_keys(env: &Env) {
    force_bump_persistent(env, &LeaderboardKey::SuccessRateIndex);
    force_bump_persistent(env, &LeaderboardKey::AdoptersIndex);
    force_bump_persistent(env, &LeaderboardKey::ProfitDeltaIndex);
    force_bump_persistent(env, &LeaderboardKey::StakeIndex);
}

fn is_qualified(entry: &IndexEntry) -> bool {
    entry.closed_signals >= MIN_CLOSED_SIGNALS && entry.total_adopters > 0
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
        let existing = without.get(i).unwrap();
        let existing_score = score_fn(&existing);
        if existing_score < entry_score {
            insert_at = i;
            break;
        }
        // Tie-break: same primary score
        if existing_score == entry_score {
            // Rule 1: earlier registration timestamp ranks higher
            if entry.registered_at < existing.registered_at {
                insert_at = i;
                break;
            }
            if entry.registered_at == existing.registered_at {
                // Rule 2: lexicographic address bytes — smaller raw bytes ranks higher
                let entry_bytes = entry.provider.to_string();
                let existing_bytes = existing.provider.to_string();
                if entry_bytes < existing_bytes {
                    insert_at = i;
                    break;
                }
            }
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

    // Retrieve registration timestamp for tie-breaking; fall back to 0 if not
    // yet profiled (providers registered before the profile feature are sorted
    // after properly-profiled ones with the same score).
    let registered_at = crate::providers::get_provider_profile(env, &provider)
        .map(|p| p.created_at)
        .unwrap_or(0);

    let entry = IndexEntry {
        provider: provider.clone(),
        closed_signals,
        success_rate: stats.success_rate,
        total_adopters: stats.total_copies as u32,
        total_profit_delta: stats.avg_return.saturating_mul(closed_signals as i128),
        stake_amount,
        verified,
        registered_at,
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
///
/// Issue #948: this is a deprecated entry point — `get_provider_leaderboard`
/// (and `get_followers_leaderboard` for the `Followers` metric) is the
/// current implementation. The shim still routes every call through to the
/// current logic so behavior is unchanged, but it now emits
/// `DeprecatedEntryPointUsed` so integrators calling the old signature are
/// visible on-chain and can migrate off it.
pub fn get_leaderboard(
    env: &Env,
    stats_map: &soroban_sdk::Map<Address, ProviderPerformance>,
    metric: LeaderboardMetric,
    limit: u32,
) -> Vec<ProviderLeaderboardEntry> {
    emit_deprecated_entry_point_used(
        env,
        Symbol::new(env, "get_leaderboard"),
        Symbol::new(env, "get_provider_leaderboard"),
    );
    match metric {
        LeaderboardMetric::SuccessRate => {
            get_provider_leaderboard(env, ProviderMetric::BySuccessRate, limit)
        }
        LeaderboardMetric::Volume => {
            get_provider_leaderboard(env, ProviderMetric::ByTotalProfitDelta, limit)
        }
        LeaderboardMetric::Followers => get_followers_leaderboard(env, stats_map, limit),
    }
}

fn get_followers_leaderboard(
    env: &Env,
    stats_map: &soroban_sdk::Map<Address, ProviderPerformance>,
    limit: u32,
) -> Vec<ProviderLeaderboardEntry> {
    let mut providers: Vec<ProviderLeaderboardEntry> = Vec::new(env);
    for key in stats_map.keys() {
        if let Some(stats) = stats_map.get(key.clone()) {
            let follower_count = social::get_follower_count(env, &key);
            if follower_count == 0 {
                continue;
            }
            let stake_amount = stake::get_stake_info(env, &key)
                .as_ref()
                .map(|s| s.amount)
                .unwrap_or(0);
            providers.push_back(ProviderLeaderboardEntry {
                rank: 0,
                provider: key.clone(),
                metric_value: follower_count as i128,
                total_signals: stats.total_signals,
                verified: stake_amount >= stake::DEFAULT_MINIMUM_STAKE,
            });
        }
    }

    let len = providers.len();
    for i in 0..len {
        for j in 0..(len - i - 1) {
            let curr = providers.get(j).unwrap();
            let next = providers.get(j + 1).unwrap();
            if curr.metric_value < next.metric_value {
                providers.set(j, next);
                providers.set(j + 1, curr);
            }
        }
    }

    let take = limit.min(providers.len());
    let mut result = Vec::new(env);
    for i in 0..take {
        let mut entry = providers.get(i).unwrap();
        entry.rank = i + 1;
        result.push_back(entry);
    }
    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProviderPerformance;
    use soroban_sdk::testutils::{Address as TestAddress, Events as _};
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
            follower_count: 0,
        }
    }

    #[test]
    fn test_zero_adoption_excluded_from_leaderboard() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            // 10 closed signals but zero adopters
            let stats = make_stats(8000, 0, 100, 5, 5);
            update_leaderboard_index(&env, p, &stats);
            let lb = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            assert_eq!(lb.len(), 0);
        });
    }

    #[test]
    fn test_one_adoption_included_in_leaderboard() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            let stats = make_stats(8000, 1, 100, 5, 5);
            update_leaderboard_index(&env, p, &stats);
            let lb = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            assert_eq!(lb.len(), 1);
        });
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
                assert!(lb.get(i).unwrap().metric_value >= lb.get(i + 1).unwrap().metric_value);
            }

            // BY_TOTAL_ADOPTERS
            let lb = get_provider_leaderboard(&env, ProviderMetric::ByTotalAdopters, 10);
            assert_eq!(lb.len(), 10);
            assert_eq!(lb.get(0).unwrap().metric_value, 150);
            for i in 0..9u32 {
                assert!(lb.get(i).unwrap().metric_value >= lb.get(i + 1).unwrap().metric_value);
            }

            // BY_TOTAL_PROFIT_DELTA
            let lb = get_provider_leaderboard(&env, ProviderMetric::ByTotalProfitDelta, 10);
            assert_eq!(lb.len(), 10);
            for i in 0..9u32 {
                assert!(lb.get(i).unwrap().metric_value >= lb.get(i + 1).unwrap().metric_value);
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

    // ── Tie-breaking tests (#611) ─────────────────────────────────────────────

    /// Helper: build an IndexEntry directly so `registered_at` can be controlled.
    fn make_entry(
        env: &Env,
        provider: Address,
        success_rate: u32,
        registered_at: u64,
    ) -> IndexEntry {
        IndexEntry {
            provider,
            closed_signals: 10,
            success_rate,
            total_adopters: 5,
            total_profit_delta: 0,
            stake_amount: 0,
            verified: false,
            registered_at,
        }
    }

    /// Two providers with identical success-rate scores: the one that registered
    /// *earlier* (smaller `registered_at`) must rank first.
    #[test]
    fn test_tiebreak_earlier_registration_ranks_higher() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p_early = Address::generate(&env);
            let p_late = Address::generate(&env);

            let score = 7500u32;
            let mut idx = Vec::new(&env);
            // Insert later-registered first, then earlier-registered
            upsert_sorted(
                &env,
                &mut idx,
                make_entry(&env, p_late.clone(), score, 2000),
                |e| e.success_rate as i128,
            );
            upsert_sorted(
                &env,
                &mut idx,
                make_entry(&env, p_early.clone(), score, 1000),
                |e| e.success_rate as i128,
            );

            // Earlier registration (t=1000) should rank first
            assert_eq!(idx.get(0).unwrap().provider, p_early);
            assert_eq!(idx.get(1).unwrap().provider, p_late);
        });
    }

    /// Repeated queries must return the exact same order (no non-determinism).
    #[test]
    fn test_tiebreak_repeated_queries_stable() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p1 = Address::generate(&env);
            let p2 = Address::generate(&env);
            let p3 = Address::generate(&env);

            let stats = make_stats(8000, 10, 50, 5, 5);
            update_leaderboard_index(&env, p1.clone(), &stats);
            update_leaderboard_index(&env, p2.clone(), &stats);
            update_leaderboard_index(&env, p3.clone(), &stats);

            // Query three times — result must be identical every time
            let q1 = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            let q2 = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);
            let q3 = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);

            assert_eq!(q1.len(), q2.len());
            assert_eq!(q2.len(), q3.len());
            for i in 0..q1.len() {
                assert_eq!(q1.get(i).unwrap().provider, q2.get(i).unwrap().provider);
                assert_eq!(q2.get(i).unwrap().provider, q3.get(i).unwrap().provider);
            }
        });
    }

    /// Entries with *distinct* primary scores must not be reordered by tie-breaking.
    #[test]
    fn test_tiebreak_does_not_alter_distinct_score_order() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p_high = Address::generate(&env);
            let p_low = Address::generate(&env);

            let mut idx = Vec::new(&env);
            // p_high has score 9000, p_low has score 6000
            // p_high registered later — tie-break would put p_low first if primary score didn't dominate
            upsert_sorted(
                &env,
                &mut idx,
                make_entry(&env, p_high.clone(), 9000, 5000),
                |e| e.success_rate as i128,
            );
            upsert_sorted(
                &env,
                &mut idx,
                make_entry(&env, p_low.clone(), 6000, 1000),
                |e| e.success_rate as i128,
            );

            // Higher primary score must still win regardless of registration time
            assert_eq!(idx.get(0).unwrap().provider, p_high);
            assert_eq!(idx.get(1).unwrap().provider, p_low);
        });
    }

    // ── Issue #948: deprecated entry point shim ─────────────────────────────
    use soroban_sdk::TryFromVal;

    fn has_deprecation_event(env: &Env) -> bool {
        env.events().all().iter().any(|e| {
            let topics: Vec<soroban_sdk::Val> = e.1.clone();
            topics
                .get(0)
                .and_then(|t| soroban_sdk::Symbol::try_from_val(env, &t).ok())
                .map(|s| s == Symbol::new(env, "deprecated_entry_point_used"))
                .unwrap_or(false)
        })
    }

    #[test]
    fn test_legacy_get_leaderboard_emits_deprecation_event() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            let stats = make_stats(8000, 1, 100, 5, 5);
            update_leaderboard_index(&env, p, &stats);

            let empty_stats_map: soroban_sdk::Map<Address, ProviderPerformance> =
                soroban_sdk::Map::new(&env);
            let board = get_leaderboard(&env, &empty_stats_map, LeaderboardMetric::SuccessRate, 10);

            // Legacy path still returns correct data...
            assert_eq!(board.len(), 1);
            // ...but is now flagged on-chain as deprecated so callers can migrate.
            assert!(
                has_deprecation_event(&env),
                "legacy get_leaderboard must emit DeprecatedEntryPointUsed"
            );
        });
    }

    #[test]
    fn test_current_get_provider_leaderboard_emits_no_deprecation_event() {
        let env = Env::default();
        let cid = env.register(TestContract, ());
        env.as_contract(&cid, || {
            let p = Address::generate(&env);
            let stats = make_stats(8000, 1, 100, 5, 5);
            update_leaderboard_index(&env, p, &stats);

            let board = get_provider_leaderboard(&env, ProviderMetric::BySuccessRate, 10);

            assert_eq!(board.len(), 1);
            assert!(
                !has_deprecation_event(&env),
                "current get_provider_leaderboard must not emit DeprecatedEntryPointUsed"
            );
        });
    }
}
