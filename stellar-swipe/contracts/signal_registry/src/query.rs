//! Active signal feed: [`get_active_signals`] (pool list / “list active signals”).
//! Hot path: sort + slice only over collected actives; avoid repeated `Map::keys()` work.

use crate::categories::SignalCategory;
use crate::reputation::get_trust_score;
use crate::social;
use crate::types::{Signal, SignalStatus, SignalSummary, SortOption};
use soroban_sdk::{contracttype, Address, Env, Map, Vec};

const MAX_LIMIT: u32 = 50;
const DEFAULT_LIMIT: u32 = 20;

/// Maximum number of records a single provider-history page may return.
/// Kept small so a single `get_provider_signal_history` call stays well
/// within Soroban read / CPU resource limits even for long histories.
pub const MAX_HISTORY_PAGE_SIZE: u32 = 50;

/// Default page size when the caller passes `limit == 0`.
pub const DEFAULT_HISTORY_PAGE_SIZE: u32 = 20;

/// One page of a provider's signal history.
///
/// The continuation value is `next_cursor`: the id of the oldest signal in
/// this page. Pass it back as `get_provider_signal_history(...).cursor` to
/// fetch the next, older page; `None` means this was the final page (no more
/// records for the provider). Because ordering is newest-first by id and the
/// cursor is an *exclusive* id bound, pages never overlap and resume
/// deterministically even if the history grows between calls.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProviderSignalHistoryPage {
    /// This page's records, newest-first by signal id. Empty for an empty
    /// history or for a cursor that is older than every signal (out-of-range).
    pub signals: Vec<Signal>,
    /// Id of the oldest signal returned; pass back as `cursor` for the next
    /// older page. `None` when there are no more records after this page.
    pub next_cursor: Option<u64>,
    /// Total number of signals this provider has (informational; can be 0).
    pub total: u32,
}

// --- Feed budget notes (Soroban `Env` + `testutils` host; 50 actives, `SortOption::RecencyDesc`) ---
// Measured in `get_active_signals_stays_under_half_default_cpu_budget_50_active`:
// `reset_tracker` → `get_active_signals` (50 actives, `RecencyDesc`, limit 30) →
// `cost_estimate().budget().cpu_instruction_cost()` (native test host; WASM will differ).
// * Before (bubble + `keys()` every index): ~62_000_000 instructions (exceeded 50% budget).
// * After (merge sort + single `keys()` snapshot): well under 50_000_000 (see test assert).
// * Protocol default CPU budget (typical tx): 100_000_000 — target < 50% = 50_000_000.

/// Read-only, bounded pagination over a single provider's signal history.
///
/// # Ordering
/// Results are **deterministic**: newest-first by signal id (ids are
/// monotonically increasing, so id order == submission order). No collection
/// larger than one page is ever allocated from the caller's page size.
///
/// # Pagination
/// `cursor` is the id of the last (oldest) signal returned by the previous
/// page, and is used as an **exclusive** lower bound, so successive pages
/// never overlap and resume even if new signals are added meanwhile. Pass
/// `None` to fetch the newest page.
///
/// # Bounded / checked behaviour
/// - `limit == 0` → [`DEFAULT_HISTORY_PAGE_SIZE`].
/// - `limit > MAX_HISTORY_PAGE_SIZE` → clamped to [`MAX_HISTORY_PAGE_SIZE`].
/// - Empty history (or a provider with no signals) → empty page with
///   `next_cursor: None` and `total: 0`.
/// - Out-of-range cursor (older than every signal) → empty page with
///   `next_cursor: None` (no subsequent data to resume from).
/// - The final page returns fewer than `limit` records with
///   `next_cursor: None` to signal the end.
pub fn get_provider_signal_history(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    provider: &Address,
    cursor: Option<u64>,
    limit: u32,
) -> ProviderSignalHistoryPage {
    // Single `keys()` snapshot; do not re-walk `keys()` per item.
    let keys = signals_map.keys();

    // Collect this provider's ids, tracking the total count along the way.
    let mut ids = Vec::new(env);
    let mut total: u32 = 0;
    for i in 0..keys.len() {
        if let Some(id) = keys.get(i) {
            if let Some(signal) = signals_map.get(id) {
                if signal.provider == *provider {
                    total = total.saturating_add(1);
                    ids.push_back(id);
                }
            }
        }
    }

    // Deterministic newest-first order (ids are monotonically increasing).
    sort_descending(env, &mut ids);

    let n = ids.len();
    if n == 0 {
        return ProviderSignalHistoryPage {
            signals: Vec::new(env),
            next_cursor: None,
            total: 0,
        };
    }

    // Bounded page size: 0 -> default, oversize -> clamp to max.
    let mut size = limit;
    if size == 0 {
        size = DEFAULT_HISTORY_PAGE_SIZE;
    } else if size > MAX_HISTORY_PAGE_SIZE {
        size = MAX_HISTORY_PAGE_SIZE;
    }

    // Find the start index: the first id strictly below `cursor` (or the very
    // newest id when `cursor` is None). Because ids are sorted descending and
    // strictly decreasing, the first id < cursor_id is exactly that index.
    let cursor_id = cursor.unwrap_or(u64::MAX);
    let mut start: u32 = 0;
    while start < n && ids.get(start).unwrap() >= cursor_id {
        start += 1;
    }

    // Take up to `size` records from `start`, building the page.
    let mut signals = Vec::new(env);
    let mut taken: u32 = 0;
    let mut idx = start;
    while idx < n && taken < size {
        let id = ids.get(idx).unwrap();
        if let Some(signal) = signals_map.get(id) {
            signals.push_back(signal);
        }
        taken += 1;
        idx += 1;
    }

    // The oldest id in this page becomes the next exclusive cursor, unless we
    // consumed every remaining record (then there is nothing left to resume).
    let next_cursor = if taken > 0 && idx < n {
        Some(ids.get(idx - 1).unwrap())
    } else {
        None
    };

    ProviderSignalHistoryPage {
        signals,
        next_cursor,
        total,
    }
}

/// In-place deterministic sort of signal ids into descending order
/// (newest-first). Signal ids are `u64`, so this is a simple numeric sort.
fn sort_descending(env: &Env, ids: &mut Vec<u64>) {
    let n = ids.len();
    if n <= 1 {
        return;
    }
    // Selection sort: adequate because each page collection only ever holds
    // this provider's ids, and histories are bounded by storage capacity.
    for i in 0..n {
        let mut max_idx = i;
        for j in (i + 1)..n {
            if ids.get(j).unwrap() > ids.get(max_idx).unwrap() {
                max_idx = j;
            }
        }
        if max_idx != i {
            let tmp = ids.get(i).unwrap();
            ids.set(i, ids.get(max_idx).unwrap());
            ids.set(max_idx, tmp);
        }
    }
}

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
    get_active_signals_internal(
        env,
        signals_map,
        provider_filter,
        offset,
        limit,
        sort_by,
        None,
    )
}

pub fn get_active_signals_personalized(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    user: Address,
    offset: u32,
    limit: u32,
    sort_by: SortOption,
    _category_filter: Option<SignalCategory>,
) -> Vec<SignalSummary> {
    get_active_signals_internal(env, signals_map, None, offset, limit, sort_by, Some(user))
}

fn get_active_signals_internal(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    provider_filter: Option<Address>,
    offset: u32,
    limit: u32,
    sort_by: SortOption,
    user: Option<Address>,
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
    sort_feed_mergesort(
        env,
        &mut active_signals,
        total_active,
        &sort_by,
        user.as_ref(),
    );

    // 3. Paginate
    let mut results = Vec::new(env);
    let end = (offset + actual_limit).min(total_active);

    for i in offset..end {
        let signal = active_signals.get(i).unwrap();
        let success_rate = (signal.successful_executions * 10_000)
            .checked_div(signal.executions)
            .unwrap_or(0);

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
fn should_swap_pair(
    env: &Env,
    curr: &Signal,
    next: &Signal,
    sort_by: &SortOption,
    user: Option<&Address>,
) -> bool {
    let curr_score = weighted_signal_score(env, curr, sort_by, user);
    let next_score = weighted_signal_score(env, next, sort_by, user);
    curr_score < next_score
}

fn weighted_signal_score(
    env: &Env,
    signal: &Signal,
    sort_by: &SortOption,
    user: Option<&Address>,
) -> i128 {
    let followed_boost = if let Some(ref u) = user {
        if social::is_following(env, u, &signal.provider) {
            1_000_000_000i128
        } else {
            0
        }
    } else {
        0
    };

    let follower_count = social::get_follower_count(env, &signal.provider) as i128;
    let trust_score = get_trust_score(env, &signal.provider)
        .map(|details| details.score as i128)
        .unwrap_or(0);

    let social_boost = follower_count.saturating_mul(5_000) + trust_score.saturating_mul(10);

    match *sort_by {
        SortOption::PerformanceDesc => {
            let success_rate = (signal.successful_executions * 10_000)
                .checked_div(signal.executions)
                .unwrap_or(0) as i128;
            success_rate * 1_000 + social_boost + followed_boost
        }
        SortOption::RecencyDesc => {
            signal.timestamp as i128 * 10_000 + social_boost + followed_boost
        }
        SortOption::VolumeDesc => signal.total_volume + social_boost * 10 + followed_boost / 100,
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
    user: Option<&Address>,
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
                if !should_swap_pair(env, &v.get(i0).unwrap(), &v.get(i1).unwrap(), sort_by, user) {
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
                let should_swap = should_swap_pair(env, &curr, &next, sort_by, None);
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
                warning_emitted: false,
                benchmark_return_bps: None,
                alpha_bps: None,
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

    fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = Env::default();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || f(&env))
    }

    #[test]
    fn get_active_signals_matches_bubble_historical_all_sorts() {
        with_contract(|env| {
            env.cost_estimate().budget().reset_unlimited();
            let map = make_test_map(env, 50);
            for sort in [
                SortOption::RecencyDesc,
                SortOption::PerformanceDesc,
                SortOption::VolumeDesc,
            ] {
                for off in [0u32, 3, 20] {
                    for lim in [0u32, 10, 25, 100] {
                        let a = get_active_signals(env, &map, None, off, lim, sort.clone(), None);
                        let b = get_active_signals_bubble_historical(
                            env, &map, None, off, lim, &sort, None,
                        );
                        assert_eq!(a.len(), b.len());
                        assert_summaries_eq(&a, &b);
                    }
                }
            }
        });
    }

    /// `cost_estimate().budget().cpu_instruction_cost()` (see module header for before/after).
    #[test]
    fn get_active_signals_stays_under_half_default_cpu_budget_50_active() {
        with_contract(|env| {
            const DEFAULT_TX_CPU: u64 = 100_000_000;
            const HALF: u64 = DEFAULT_TX_CPU / 2;
            let map = make_test_map(env, 50);
            env.cost_estimate().budget().reset_tracker();
            let _ = get_active_signals(env, &map, None, 0, 30, SortOption::RecencyDesc, None);
            let after = env.cost_estimate().budget().cpu_instruction_cost();
            assert!(
                after < HALF,
                "get_active_signals(50 actives) used {after} insns, expected < {HALF} (50% of {DEFAULT_TX_CPU})"
            );
        });
    }
}

#[cfg(test)]
mod provider_history_tests {
    use super::*;
    use crate::categories::RiskLevel;
    use crate::types::SignalAction;
    use core::assert_eq;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::String;

    fn make_signal(env: &Env, id: u64, provider: &Address) -> Signal {
        Signal {
            id,
            provider: provider.clone(),
            asset_pair: String::from_str(env, "XLM-USDC"),
            action: if id % 2 == 0 {
                SignalAction::Buy
            } else {
                SignalAction::Sell
            },
            price: 1_000_000 + id as i128 * 1_000,
            rationale: String::from_str(env, "q"),
            timestamp: 1_000_000 + id,
            expiry: 1_000_000_000,
            status: SignalStatus::Active,
            executions: 1,
            successful_executions: 1,
            total_volume: 1000 * id as i128,
            total_roi: 0,
            category: crate::categories::SignalCategory::SWING,
            tags: soroban_sdk::vec![env, String::from_str(env, "a")],
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
            submitted_at: 1_000_000 + id,
            rationale_hash: String::from_str(env, "q"),
            confidence: 50,
            adoption_count: 0,
            ai_validation_score: None,
            avg_copier_roi_bps: 0,
            copier_closed_count: 0,
            warning_emitted: false,
            benchmark_return_bps: None,
            alpha_bps: None,
        }
    }

    /// Build a map with `ids` assigned to `provider` plus a few signals from
    /// `other` (used to prove cross-provider filtering).
    fn make_map(env: &Env, provider: &Address, other: &Address, ids: Vec<u64>) -> Map<u64, Signal> {
        let mut m = Map::new(env);
        for i in 0..ids.len() {
            let id = ids.get(i).unwrap();
            m.set(id, make_signal(env, id, provider));
        }
        // Two signals from a different provider; must never leak into results.
        m.set(10_000, make_signal(env, 10_000, other));
        m.set(10_001, make_signal(env, 10_001, other));
        m
    }

    fn ids_of(env: &Env, page: &ProviderSignalHistoryPage) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::new(env);
        for i in 0..page.signals.len() {
            out.push_back(page.signals.get(i).unwrap().id);
        }
        out
    }

    fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = Env::default();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || f(&env))
    }

    #[test]
    fn empty_history_returns_empty_page() {
        with_contract(|env| {
            let provider = Address::generate(env);
            let other = Address::generate(env);
            let mut map = Map::new(env);
            // Only the other provider's signals exist; `provider` is empty.
            for id in [10_000u64, 10_001] {
                map.set(id, make_signal(env, id, &other));
            }
            let page = get_provider_signal_history(env, &map, &provider, None, 10);
            assert_eq!(page.signals.len(), 0);
            assert_eq!(page.next_cursor, None);
            assert_eq!(page.total, 0);
        });
    }

    #[test]
    fn single_page_fits_in_one_request() {
        with_contract(|env| {
            let provider = Address::generate(env);
            let other = Address::generate(env);
            // ids 1..=5 → newest-first should be [5,4,3,2,1].
            let ids = soroban_sdk::vec![env, 1u64, 2, 3, 4, 5];
            let map = make_map(env, &provider, &other, ids);

            let page = get_provider_signal_history(env, &map, &provider, None, 20);
            assert_eq!(page.total, 5);
            assert_eq!(page.next_cursor, None, "single page consumes all records");
            let got = ids_of(env, &page);
            assert_eq!(got.len(), 5);
            assert_eq!(got.get(0).unwrap(), 5);
            assert_eq!(got.get(1).unwrap(), 4);
            assert_eq!(got.get(2).unwrap(), 3);
            assert_eq!(got.get(3).unwrap(), 2);
            assert_eq!(got.get(4).unwrap(), 1);
        });
    }

    #[test]
    fn multi_page_walks_newest_to_oldest_without_overlap() {
        with_contract(|env| {
            let provider = Address::generate(env);
            let other = Address::generate(env);
            // ids 1..=7 with a small page size → pages [2 items each].
            let ids = soroban_sdk::vec![env, 1u64, 2, 3, 4, 5, 6, 7];
            let map = make_map(env, &provider, &other, ids);

            let mut cursor = None;
            let mut collected: Vec<u64> = Vec::new(env);
            let mut pages = 0;
            loop {
                pages += 1;
                let page = get_provider_signal_history(env, &map, &provider, cursor, 2);
                assert!(page.signals.len() > 0, "every page is non-empty");
                // Any page that still has a continuation must be exactly full;
                // only the final page may be shorter than the page size.
                if page.next_cursor.is_some() {
                    assert_eq!(page.signals.len(), 2, "non-final page is full");
                } else {
                    assert!(page.signals.len() <= 2, "final page is at most page size");
                }
                for i in 0..page.signals.len() {
                    collected.push_back(page.signals.get(i).unwrap().id);
                }
                match page.next_cursor {
                    Some(c) => cursor = Some(c),
                    None => break,
                }
            }

            // ids 7..1 newest-first, no overlap, fully drained.
            assert_eq!(pages, 4);
            assert_eq!(collected.len(), 7);
            for k in 0..collected.len() {
                assert_eq!(collected.get(k).unwrap(), (7 - k as u64), "idx {k}");
            }
        });
    }

    #[test]
    fn page_size_clamped_to_max_and_default() {
        with_contract(|env| {
            let provider = Address::generate(env);
            let other = Address::generate(env);
            let ids = soroban_sdk::vec![env, 1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            let map = make_map(env, &provider, &other, ids);

            // limit == 0 → DEFAULT_HISTORY_PAGE_SIZE.
            let page = get_provider_signal_history(env, &map, &provider, None, 0);
            assert!(page.signals.len() <= DEFAULT_HISTORY_PAGE_SIZE);
            assert!(page.signals.len() > 0);

            // Oversized request is clamped to MAX_HISTORY_PAGE_SIZE.
            let oversized =
                get_provider_signal_history(env, &map, &provider, None, 10_000);
            assert!(oversized.signals.len() <= MAX_HISTORY_PAGE_SIZE);
        });
    }

    #[test]
    fn out_of_range_cursor_returns_empty_page() {
        with_contract(|env| {
            let provider = Address::generate(env);
            let other = Address::generate(env);
            let ids = soroban_sdk::vec![env, 1u64, 2, 3];
            let map = make_map(env, &provider, &other, ids);

            // Cursor older than the oldest signal → no records and no resume.
            let page = get_provider_signal_history(env, &map, &provider, Some(0), 10);
            assert_eq!(page.signals.len(), 0);
            assert_eq!(page.next_cursor, None);
            assert_eq!(page.total, 3);
        });
    }

    #[test]
    fn other_providers_signals_are_excluded() {
        with_contract(|env| {
            let provider = Address::generate(env);
            let other = Address::generate(env);
            let ids = soroban_sdk::vec![env, 5u64, 6, 7];
            let map = make_map(env, &provider, &other, ids);

            let page = get_provider_signal_history(env, &map, &provider, None, 10);
            // other's ids 10000/10001 must not appear.
            for i in 0..page.signals.len() {
                let id = page.signals.get(i).unwrap().id;
                assert!(id <= 7, "unexpected foreign signal {id}");
            }
            assert_eq!(page.total, 3);
        });
    }
}
