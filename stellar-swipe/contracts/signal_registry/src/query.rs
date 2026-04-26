//! Active signal feed: [`get_active_signals`] (pool list / “list active signals”).
//! Hot path: sort + slice only over collected actives; avoid repeated `Map::keys()` work.

use crate::categories::SignalCategory;
use crate::types::{Signal, SignalStatus, SignalSummary, SortOption};
use soroban_sdk::{Address, Env, Map, Vec};

const MAX_LIMIT: u32 = 50;
const DEFAULT_LIMIT: u32 = 20;

// --- Feed budget notes (Soroban `Env` + `testutils` host; 50 actives, `SortOption::RecencyDesc`) ---
// Measured in `get_active_signals_stays_under_half_default_cpu_budget_50_active`:
// `reset_tracker` → `get_active_signals` (50 actives, `RecencyDesc`, limit 30) →
// `cost_estimate().budget().cpu_instruction_cost()` (native test host; WASM will differ).
// * Before (bubble + `keys()` every index): ~62_000_000 instructions (exceeded 50% budget).
// * After (merge sort + single `keys()` snapshot): well under 50_000_000 (see test assert).
// * Protocol default CPU budget (typical tx): 100_000_000 — target < 50% = 50_000_000.

/// Implement Batch Signal Querying & Feed Pagination
pub fn get_active_signals(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    provider_filter: Option<Address>,
    offset: u32,
    limit: u32,
    sort_by: SortOption,
    _category_filter: Option<SignalCategory>,
) -> Vec<SignalSummary> {
    let mut active_signals = Vec::new(env);
    let current_time = env.ledger().timestamp();

    // A single `keys()` snapshot; the previous pattern called `keys()` per loop iteration
    // (repeated map walks / host work).
    let key_list = signals_map.keys();
    let n_keys = key_list.len();
    for i in 0..n_keys {
        if let Some(key) = key_list.get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.expiry > current_time
                    && signal.status != SignalStatus::Expired
                    && signal.status != SignalStatus::Executed
                {
                    let include = if let Some(ref p) = provider_filter {
                        signal.provider == *p
                    } else {
                        true
                    };
                    if include {
                        active_signals.push_back(signal);
                    }
                }
            }
        }
    }

    let total_active = active_signals.len();

    // If offset is beyond count or no signals, return empty
    if offset >= total_active || total_active == 0 {
        return Vec::new(env);
    }

    // Clamp limit
    let mut actual_limit = limit;
    if actual_limit == 0 {
        actual_limit = DEFAULT_LIMIT;
    } else if actual_limit > MAX_LIMIT {
        actual_limit = MAX_LIMIT;
    }

    // 2. Sort: bottom-up merge sort, same order as historical bubble/insertion (O(n log n) passes).
    sort_feed_mergesort(env, &mut active_signals, total_active, &sort_by);

    // 3. Paginate
    let mut results = Vec::new(env);
    let end = (offset + actual_limit).min(total_active);

    for i in offset..end {
        let signal = active_signals.get(i).unwrap();
        let success_rate = if signal.executions > 0 {
            (signal.successful_executions * 10_000) / signal.executions
        } else {
            0
        };

        results.push_back(SignalSummary {
            id: signal.id,
            provider: signal.provider,
            asset_pair: signal.asset_pair,
            action: signal.action,
            price: signal.price,
            success_rate,
            total_copies: signal.executions,
            timestamp: signal.timestamp,
        });
    }

    results
}

/// Same as historical bubble: returns true if **left** should move right (swap with **right**).
fn should_swap_pair(curr: &Signal, next: &Signal, sort_by: &SortOption) -> bool {
    match *sort_by {
        SortOption::PerformanceDesc => {
            let curr_success = if curr.executions > 0 {
                (curr.successful_executions * 10_000) / curr.executions
            } else {
                0
            };
            let next_success = if next.executions > 0 {
                (next.successful_executions * 10_000) / next.executions
            } else {
                0
            };
            if curr_success < next_success {
                true
            } else if curr_success == next_success {
                curr.timestamp < next.timestamp
            } else {
                false
            }
        }
        SortOption::RecencyDesc => curr.timestamp < next.timestamp,
        SortOption::VolumeDesc => {
            if curr.total_volume < next.total_volume {
                true
            } else if curr.total_volume == next.total_volume {
                curr.timestamp < next.timestamp
            } else {
                false
            }
        }
    }
}

/// In-place (buffered) bottom-up merge sort. Uses the same pairwise predicate as bubble/insertion.
/// Avoids O(n^2) bubble/insertion cost on the active feed, which is the dominant part of
/// `get_active_signals` for max-sized maps.
fn sort_feed_mergesort(
    env: &Env,
    v: &mut Vec<Signal>,
    n: u32,
    sort_by: &SortOption,
) {
    if n <= 1 {
        return;
    }
    let mut w: u32 = 1;
    while w < n {
        let mut nxt: Vec<Signal> = Vec::new(env);
        let mut st: u32 = 0;
        while st < n {
            let m = (st + w).min(n);
            let e = (st + (2 * w)).min(n);
            let mut i0 = st;
            let mut i1 = m;
            while i0 < m && i1 < e {
                if !should_swap_pair(
                    &v.get(i0).unwrap(),
                    &v.get(i1).unwrap(),
                    sort_by,
                ) {
                    nxt.push_back(v.get(i0).unwrap());
                    i0 += 1;
                } else {
                    nxt.push_back(v.get(i1).unwrap());
                    i1 += 1;
                }
            }
            while i0 < m {
                nxt.push_back(v.get(i0).unwrap());
                i0 += 1;
            }
            while i1 < e {
                nxt.push_back(v.get(i1).unwrap());
                i1 += 1;
            }
            st = e;
        }
        for i in 0..n {
            v.set(i, nxt.get(i).unwrap());
        }
        w = w * 2;
    }
}

#[cfg(test)]
mod feed_tests {
    use super::*;
    use core::assert_eq;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::String;

    /// Historical implementation (pre-optimization): per-iter `keys()` + bubble sort. Used
    /// only to verify identical `SignalSummary` output to [`super::get_active_signals`].
    fn get_active_signals_bubble_historical(
        env: &Env,
        signals_map: &Map<u64, Signal>,
        provider_filter: Option<Address>,
        offset: u32,
        limit: u32,
        sort_by: &SortOption,
        _category_filter: Option<SignalCategory>,
    ) -> Vec<SignalSummary> {
        let mut active_signals = Vec::new(env);
        let current_time = env.ledger().timestamp();
        for i in 0..signals_map.keys().len() {
            if let Some(key) = signals_map.keys().get(i) {
                if let Some(signal) = signals_map.get(key) {
                    if signal.expiry > current_time
                        && signal.status != SignalStatus::Expired
                        && signal.status != SignalStatus::Executed
                    {
                        let mut include = true;
                        if let Some(ref p) = provider_filter {
                            if signal.provider != *p {
                                include = false;
                            }
                        }
                        if include {
                            active_signals.push_back(signal);
                        }
                    }
                }
            }
        }

        let total_active = active_signals.len();
        if offset >= total_active || total_active == 0 {
            return Vec::new(env);
        }
        let mut actual_limit = limit;
        if actual_limit == 0 {
            actual_limit = DEFAULT_LIMIT;
        } else if actual_limit > MAX_LIMIT {
            actual_limit = MAX_LIMIT;
        }
        for i in 0..total_active {
            for j in 0..(total_active - i - 1) {
                let curr = active_signals.get(j).unwrap();
                let next = active_signals.get(j + 1).unwrap();
                let should_swap = should_swap_pair(&curr, &next, sort_by);
                if should_swap {
                    active_signals.set(j, next);
                    active_signals.set(j + 1, curr);
                }
            }
        }
        let mut results = Vec::new(env);
        let end = (offset + actual_limit).min(total_active);
        for i in offset..end {
            let signal = active_signals.get(i).unwrap();
            let success_rate = if signal.executions > 0 {
                (signal.successful_executions * 10_000) / signal.executions
            } else {
                0
            };
            results.push_back(SignalSummary {
                id: signal.id,
                provider: signal.provider,
                asset_pair: signal.asset_pair,
                action: signal.action,
                price: signal.price,
                success_rate,
                total_copies: signal.executions,
                timestamp: signal.timestamp,
            });
        }
        results
    }

    fn make_test_map(env: &Env, n: u32) -> Map<u64, Signal> {
        use crate::categories::RiskLevel;
        use crate::types::SignalAction;
        let mut m = Map::new(env);
        let p = Address::generate(env);
        let t0 = 1_000_000u64;
        for i in 0..n {
            let id = (i as u64) + 1;
            let s = Signal {
                id,
                provider: p.clone(),
                asset_pair: String::from_str(env, "XLM-USDC"),
                action: if id % 2 == 0 {
                    SignalAction::Buy
                } else {
                    SignalAction::Sell
                },
                price: 1_000_000 + id as i128 * 1_000,
                rationale: String::from_str(env, "q"),
                timestamp: t0 + (id * 3) % 500,
                expiry: t0 + 86_400_000,
                status: SignalStatus::Active,
                executions: 1 + (id as u32 % 7),
                successful_executions: (id as u32 % 5) + 1,
                total_volume: 1000 * (id as i128),
                total_roi: 0,
                category: crate::categories::SignalCategory::SWING,
                tags: soroban_sdk::vec![env, String::from_str(env, "a")],
                risk_level: RiskLevel::Medium,
                is_collaborative: false,
                submitted_at: t0,
                rationale_hash: String::from_str(env, "q"),
                confidence: 50,
                adoption_count: 0,
                ai_validation_score: None,
                avg_copier_roi_bps: 0,
                copier_closed_count: 0,
            };
            m.set(id, s);
        }
        m
    }

    fn assert_summaries_eq(a: &Vec<SignalSummary>, b: &Vec<SignalSummary>) {
        assert_eq!(a.len(), b.len());
        for k in 0..a.len() {
            let x = a.get(k).unwrap();
            let y = b.get(k).unwrap();
            assert_eq!(x.id, y.id, "k={k}");
            assert_eq!(x.provider, y.provider, "k={k}");
            assert_eq!(x.asset_pair, y.asset_pair, "k={k}");
            assert_eq!(x.action, y.action, "k={k}");
            assert_eq!(x.price, y.price, "k={k}");
            assert_eq!(x.success_rate, y.success_rate, "k={k}");
            assert_eq!(x.total_copies, y.total_copies, "k={k}");
            assert_eq!(x.timestamp, y.timestamp, "k={k}");
        }
    }

    #[test]
    fn get_active_signals_matches_bubble_historical_all_sorts() {
        let env = Env::default();
        // Many param combinations + historical O(n^2) reference can exceed default test budget.
        env.cost_estimate().budget().reset_unlimited();
        let map = make_test_map(&env, 50);
        for sort in [
            SortOption::RecencyDesc,
            SortOption::PerformanceDesc,
            SortOption::VolumeDesc,
        ] {
            for off in [0u32, 3, 20] {
                for lim in [0u32, 10, 25, 100] {
                    let a = get_active_signals(
                        &env, &map, None, off, lim, sort.clone(), None,
                    );
                    let b = get_active_signals_bubble_historical(
                        &env, &map, None, off, lim, &sort, None,
                    );
                    assert_eq!(a.len(), b.len());
                    assert_summaries_eq(&a, &b);
                }
            }
        }
    }

    /// `cost_estimate().budget().cpu_instruction_cost()` (see module header for before/after).
    #[test]
    fn get_active_signals_stays_under_half_default_cpu_budget_50_active() {
        const DEFAULT_TX_CPU: u64 = 100_000_000;
        const HALF: u64 = DEFAULT_TX_CPU / 2;
        let env = Env::default();
        let map = make_test_map(&env, 50);
        env.cost_estimate().budget().reset_tracker();
        let _ = get_active_signals(
            &env,
            &map,
            None,
            0,
            30,
            SortOption::RecencyDesc,
            None,
        );
        let after = env.cost_estimate().budget().cpu_instruction_cost();
        // Re-run with `cargo test get_active_signals_stays_under_half -- --nocapture` to log for PRs.
        assert!(
            after < HALF,
            "get_active_signals(50 actives) used {after} insns, expected < {HALF} (50% of {DEFAULT_TX_CPU})"
        );
    }
}
