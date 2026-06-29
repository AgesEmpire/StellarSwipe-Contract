use crate::categories::SignalCategory;
use crate::social::get_follower_count;
use crate::types::{Signal, SignalStatus};
use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};
use stellar_swipe_common::{DEFAULT_INSTRUCTION_BUDGET, SECONDS_PER_DAY, SECONDS_PER_HOUR};

const MIN_SIGNALS_FOR_ANALYTICS: u32 = 10;

#[contracttype]
#[derive(Clone, Debug)]
pub struct ProviderAnalytics {
    pub provider: Address,
    pub total_signals: u32,
    pub avg_roi: i128,
    pub best_asset_pair: String,
    pub best_time_of_day: u32,
    pub win_streak: u32,
    pub avg_signal_lifetime: u64,
    pub follower_growth_rate: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalAnalytics {
    pub total_signals_24h: u32,
    pub most_traded_pairs: Vec<(String, u32)>,
    pub avg_success_rate: u32,
    pub total_volume_24h: i128,
}

pub fn calculate_provider_analytics(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    provider: &Address,
) -> Option<ProviderAnalytics> {
    let signals = get_provider_signals(signals_map, provider);
    let total = signals.len();

    if total < MIN_SIGNALS_FOR_ANALYTICS {
        return None;
    }

    let avg_roi = calculate_avg_roi(&signals);
    let best_asset_pair = find_best_asset_pair(env, &signals);
    let best_time_of_day = find_best_time_of_day(&signals);
    let win_streak = calculate_win_streak(&signals);
    let avg_signal_lifetime = calculate_avg_lifetime(&signals);
    let follower_growth_rate = calculate_follower_growth(env, provider);

    Some(ProviderAnalytics {
        provider: provider.clone(),
        total_signals: total,
        avg_roi,
        best_asset_pair,
        best_time_of_day,
        win_streak,
        avg_signal_lifetime,
        follower_growth_rate,
    })
}

pub fn get_trending_assets(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    window_hours: u64,
) -> Vec<(String, u32)> {
    let cutoff = env
        .ledger()
        .timestamp()
        .saturating_sub(window_hours * SECONDS_PER_HOUR);
    let mut pair_counts: Map<String, u32> = Map::new(env);

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.timestamp >= cutoff {
                    let count = pair_counts.get(signal.asset_pair.clone()).unwrap_or(0);
                    pair_counts.set(signal.asset_pair.clone(), count + 1);
                }
            }
        }
    }

    let mut sorted = Vec::new(env);
    for i in 0..pair_counts.keys().len() {
        if let Some(key) = pair_counts.keys().get(i) {
            if let Some(count) = pair_counts.get(key.clone()) {
                sorted.push_back((key, count));
            }
        }
    }

    // Sort descending by count
    for i in 0..sorted.len() {
        for j in 0..(sorted.len().saturating_sub(i + 1)) {
            let curr = sorted.get(j).unwrap();
            let next = sorted.get(j + 1).unwrap();
            if curr.1 < next.1 {
                sorted.set(j, next);
                sorted.set(j + 1, curr);
            }
        }
    }

    let mut result = Vec::new(env);
    for i in 0..sorted.len().min(10) {
        result.push_back(sorted.get(i).unwrap());
    }
    result
}

pub fn calculate_global_analytics(env: &Env, signals_map: &Map<u64, Signal>) -> GlobalAnalytics {
    let cutoff = env.ledger().timestamp().saturating_sub(SECONDS_PER_DAY);
    let mut total_signals_24h = 0u32;
    let mut total_volume_24h = 0i128;
    let mut successful = 0u32;
    let mut terminal = 0u32;

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.timestamp >= cutoff {
                    total_signals_24h += 1;
                    total_volume_24h = total_volume_24h.saturating_add(signal.total_volume);
                }
                if matches!(
                    signal.status,
                    SignalStatus::Successful | SignalStatus::Failed
                ) && signal.adoption_count > 0
                {
                    terminal += 1;
                    if signal.status == SignalStatus::Successful {
                        successful += 1;
                    }
                }
            }
        }
    }

    let avg_success_rate = if terminal > 0 {
        (successful * 10000) / terminal
    } else {
        0
    };

    GlobalAnalytics {
        total_signals_24h,
        most_traded_pairs: get_trending_assets(env, signals_map, 24),
        avg_success_rate,
        total_volume_24h,
    }
}

fn get_provider_signals(signals_map: &Map<u64, Signal>, provider: &Address) -> Vec<Signal> {
    let env = signals_map.env();
    let mut result = Vec::new(&env);

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.provider == *provider {
                    result.push_back(signal);
                }
            }
        }
    }
    result
}

fn calculate_avg_roi(signals: &Vec<Signal>) -> i128 {
    if signals.is_empty() {
        return 0;
    }

    let mut total = 0i128;
    let mut count = 0u32;

    for i in 0..signals.len() {
        let signal = signals.get(i).unwrap();
        if signal.executions > 0 {
            total = total.saturating_add(signal.total_roi / signal.executions as i128);
            count += 1;
        }
    }

    if count > 0 {
        total / count as i128
    } else {
        0
    }
}

fn find_best_asset_pair(env: &Env, signals: &Vec<Signal>) -> String {
    let mut pair_roi: Map<String, i128> = Map::new(env);

    for i in 0..signals.len() {
        let signal = signals.get(i).unwrap();
        if signal.executions > 0 {
            let roi = signal.total_roi / signal.executions as i128;
            let current = pair_roi.get(signal.asset_pair.clone()).unwrap_or(0);
            pair_roi.set(signal.asset_pair.clone(), current + roi);
        }
    }

    let mut best_pair = String::from_str(env, "");
    let mut best_roi = i128::MIN;

    for i in 0..pair_roi.keys().len() {
        if let Some(key) = pair_roi.keys().get(i) {
            if let Some(roi) = pair_roi.get(key.clone()) {
                if roi > best_roi {
                    best_roi = roi;
                    best_pair = key;
                }
            }
        }
    }

    best_pair
}

fn find_best_time_of_day(signals: &Vec<Signal>) -> u32 {
    let mut hour_roi = [0i128; 24];
    let mut hour_counts = [0u32; 24];

    for i in 0..signals.len() {
        let signal = signals.get(i).unwrap();
        if signal.executions > 0 {
            let hour = ((signal.timestamp % 86400) / 3600) as usize;
            if hour < 24 {
                hour_roi[hour] =
                    hour_roi[hour].saturating_add(signal.total_roi / signal.executions as i128);
                hour_counts[hour] += 1;
            }
        }
    }

    let mut best_hour = 0u32;
    let mut best_avg = i128::MIN;

    for h in 0..24 {
        if hour_counts[h] > 0 {
            let avg = hour_roi[h] / hour_counts[h] as i128;
            if avg > best_avg {
                best_avg = avg;
                best_hour = h as u32;
            }
        }
    }

    best_hour
}

fn calculate_win_streak(signals: &Vec<Signal>) -> u32 {
    let mut streak = 0u32;
    let mut max_streak = 0u32;

    for i in 0..signals.len() {
        let signal = signals.get(i).unwrap();
        if signal.status == SignalStatus::Successful {
            streak += 1;
            if streak > max_streak {
                max_streak = streak;
            }
        } else if signal.status == SignalStatus::Failed {
            streak = 0;
        }
    }

    max_streak
}

fn calculate_avg_lifetime(signals: &Vec<Signal>) -> u64 {
    if signals.is_empty() {
        return 0;
    }

    let mut total = 0u64;
    for i in 0..signals.len() {
        let signal = signals.get(i).unwrap();
        total = total.saturating_add(signal.expiry.saturating_sub(signal.timestamp));
    }

    total / signals.len() as u64
}

fn calculate_follower_growth(env: &Env, provider: &Address) -> i128 {
    // Simplified: return current follower count as growth rate
    // Full implementation would track historical data
    get_follower_count(env, provider) as i128
}

// ═══════════════════════════════════════════════════════════════════
// Issue #419: Signal Category Performance Analytics
// ═══════════════════════════════════════════════════════════════════

#[contracttype]
#[derive(Clone, Debug)]
pub struct CategoryAnalytics {
    /// Average success rate across all closed signals in the category (bps)
    pub avg_success_rate: u32,
    /// Average ROI in basis points across all closed signals
    pub avg_roi_bps: i128,
    /// Total number of signals in this category (all statuses)
    pub total_signals: u32,
    /// Total unique adopters across signals in this category
    pub total_adopters: u32,
    /// Address of the top provider by success rate in this category (empty string if none)
    pub top_provider: Address,
}

/// Aggregate category analytics from all closed signals.
/// Returns zero-valued analytics for empty categories (no error).
pub fn calculate_category_analytics(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    category: &SignalCategory,
) -> CategoryAnalytics {
    let mut total_signals: u32 = 0;
    let mut total_successful: u32 = 0;
    let mut closed_count: u32 = 0;
    let mut total_roi: i128 = 0;
    let mut total_adopters: u32 = 0;
    let mut provider_success: Map<Address, (u32, u32)> = Map::new(env); // (successful, total)

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.category != *category {
                    continue;
                }
                total_signals += 1;
                total_adopters = total_adopters.saturating_add(signal.adoption_count);

                // Only count closed (terminal) signals with adoption for success rate
                if matches!(
                    signal.status,
                    SignalStatus::Successful | SignalStatus::Failed
                ) && signal.adoption_count > 0
                {
                    closed_count += 1;
                    if signal.status == SignalStatus::Successful {
                        total_successful += 1;
                    }

                    // Track per-provider stats for top_provider
                    let entry = provider_success
                        .get(signal.provider.clone())
                        .unwrap_or((0, 0));
                    let new_successful = if signal.status == SignalStatus::Successful {
                        entry.0 + 1
                    } else {
                        entry.0
                    };
                    provider_success.set(signal.provider.clone(), (new_successful, entry.1 + 1));

                    // Accumulate average ROI per signal
                    if signal.executions > 0 {
                        total_roi =
                            total_roi.saturating_add(signal.total_roi / signal.executions as i128);
                    }
                }
            }
        }
    }

    let avg_success_rate = if closed_count > 0 {
        (total_successful * 10000) / closed_count
    } else {
        0
    };

    let avg_roi_bps = if closed_count > 0 {
        total_roi / closed_count as i128
    } else {
        0
    };

    // Find top provider (highest success rate among those with >= 3 closed signals)
    let mut top_provider: Option<Address> = None;
    let mut top_rate: u32 = 0;
    for key in provider_success.keys() {
        if let Some((successful, total)) = provider_success.get(key.clone()) {
            if total >= 3 {
                let rate = (successful * 10000) / total;
                if rate > top_rate {
                    top_rate = rate;
                    top_provider = Some(key);
                }
            }
        }
    }

    CategoryAnalytics {
        avg_success_rate,
        avg_roi_bps,
        total_signals,
        total_adopters,
        top_provider: top_provider.unwrap_or_else(|| {
            // Use a zero-address placeholder when no provider qualifies
            Address::from_str(
                env,
                "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
            )
        }),
    }
}

// ═══════════════════════════════════════════════════════════════════
// Issue #598: Budget-aware pagination for analytics_engine queries
// ═══════════════════════════════════════════════════════════════════
//
// `calculate_global_analytics` above scans the entire `signals_map` in one
// call with no awareness of the instruction budget, so as the number of
// signals grows it risks a mid-query budget failure instead of a clean
// error. Soroban contracts cannot read the host's *remaining* instruction
// budget at runtime, so this uses a conservative estimated-cost-per-signal
// model (calibrated relative to `DEFAULT_INSTRUCTION_BUDGET`) to decide when
// to stop a page early and hand back a cursor, rather than walking the
// whole map unconditionally.

/// Conservative estimated CPU instructions consumed per signal scanned.
/// Deliberately pessimistic: overestimating just means we paginate a bit
/// earlier than strictly necessary, which is the safe direction to err in.
const EST_INSTRUCTIONS_PER_SIGNAL: u64 = 50_000;

/// Re-check the running cost estimate every `BUDGET_CHECK_INTERVAL` signals
/// instead of every single one, so the check itself stays cheap.
const BUDGET_CHECK_INTERVAL: u32 = 20;

/// Stop pulling more signals once the running estimate reaches this percent
/// of the default instruction budget, leaving headroom in the transaction
/// for whatever the caller does with the result (events, storage, etc).
const BUDGET_SAFETY_MARGIN_PCT: u64 = 60;

/// Resumable accumulator for [`calculate_global_analytics_paginated`].
/// Pass `GlobalAnalyticsAccumulator::new()` on the first call, then feed the
/// returned accumulator back in on each subsequent call until `cursor` is
/// `None`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalAnalyticsAccumulator {
    pub total_signals_24h: u32,
    pub total_volume_24h: i128,
    pub successful: u32,
    pub terminal: u32,
}

impl GlobalAnalyticsAccumulator {
    pub fn new() -> Self {
        GlobalAnalyticsAccumulator {
            total_signals_24h: 0,
            total_volume_24h: 0,
            successful: 0,
            terminal: 0,
        }
    }

    /// Average success rate in bps, matching `calculate_global_analytics`'s formula.
    pub fn avg_success_rate(&self) -> u32 {
        if self.terminal > 0 {
            (self.successful * 10000) / self.terminal
        } else {
            0
        }
    }
}

/// Result of one page of [`calculate_global_analytics_paginated`].
#[contracttype]
#[derive(Clone, Debug)]
pub struct PagedGlobalAnalytics {
    pub accumulator: GlobalAnalyticsAccumulator,
    /// Signal id to pass back in as `cursor` on the next call. `None` once
    /// the whole map has been covered — accumulation is complete.
    pub cursor: Option<u64>,
}

/// Budget-aware, resumable version of [`calculate_global_analytics`]'s 24h
/// signal-count/volume/success-rate aggregation.
///
/// Resumes scanning just after `cursor` (`None` to start from the
/// beginning), accumulating into `acc`, and stops *before* the estimated
/// instruction cost crosses `BUDGET_SAFETY_MARGIN_PCT` of
/// `DEFAULT_INSTRUCTION_BUDGET` — returning a `cursor` for the caller to
/// resume from on a subsequent call instead of risking a mid-query failure.
/// Datasets small enough to finish under the margin in one pass return
/// `cursor: None` immediately, same as a single unpaginated call.
pub fn calculate_global_analytics_paginated(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    cursor: Option<u64>,
    mut acc: GlobalAnalyticsAccumulator,
) -> PagedGlobalAnalytics {
    let cutoff = env.ledger().timestamp().saturating_sub(SECONDS_PER_DAY);
    let keys = signals_map.keys();
    let n = keys.len();

    let mut start_idx: u32 = 0;
    if let Some(after_key) = cursor {
        for i in 0..n {
            if keys.get(i) == Some(after_key) {
                start_idx = i.saturating_add(1);
                break;
            }
        }
    }

    let budget_limit = DEFAULT_INSTRUCTION_BUDGET.saturating_mul(BUDGET_SAFETY_MARGIN_PCT) / 100;
    let mut estimated_instructions: u64 = 0;
    let mut processed_in_page: u32 = 0;

    let mut i = start_idx;
    while i < n {
        if let Some(key) = keys.get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.timestamp >= cutoff {
                    acc.total_signals_24h = acc.total_signals_24h.saturating_add(1);
                    acc.total_volume_24h =
                        acc.total_volume_24h.saturating_add(signal.total_volume);
                }
                if matches!(
                    signal.status,
                    SignalStatus::Successful | SignalStatus::Failed
                ) && signal.adoption_count > 0
                {
                    acc.terminal = acc.terminal.saturating_add(1);
                    if signal.status == SignalStatus::Successful {
                        acc.successful = acc.successful.saturating_add(1);
                    }
                }
            }

            estimated_instructions =
                estimated_instructions.saturating_add(EST_INSTRUCTIONS_PER_SIGNAL);
            processed_in_page = processed_in_page.saturating_add(1);

            if processed_in_page % BUDGET_CHECK_INTERVAL == 0
                && estimated_instructions >= budget_limit
            {
                return PagedGlobalAnalytics {
                    accumulator: acc,
                    cursor: Some(key),
                };
            }
        }
        i += 1;
    }

    PagedGlobalAnalytics {
        accumulator: acc,
        cursor: None,
    }
}

#[cfg(test)]
mod pagination_tests {
    use super::*;
    use crate::categories::RiskLevel;
    use crate::types::SignalAction;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn make_signal(env: &Env, id: u64, provider: &Address, ts: u64) -> Signal {
        Signal {
            id,
            provider: provider.clone(),
            asset_pair: String::from_str(env, "XLM-USDC"),
            action: SignalAction::Buy,
            price: 1_000_000,
            rationale: String::from_str(env, "q"),
            timestamp: ts,
            expiry: ts + 86_400,
            status: if id % 3 == 0 {
                SignalStatus::Successful
            } else if id % 3 == 1 {
                SignalStatus::Failed
            } else {
                SignalStatus::Active
            },
            executions: 1,
            successful_executions: 1,
            total_volume: 1_000,
            total_roi: 0,
            category: SignalCategory::SWING,
            tags: soroban_sdk::vec![env, String::from_str(env, "a")],
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
            submitted_at: ts,
            rationale_hash: String::from_str(env, "q"),
            confidence: 50,
            adoption_count: 1,
            ai_validation_score: None,
            avg_copier_roi_bps: 0,
            copier_closed_count: 0,
            warning_emitted: false,
            benchmark_return_bps: None,
            alpha_bps: None,
        }
    }

    fn make_map(env: &Env, n: u64, ts: u64) -> Map<u64, Signal> {
        env.cost_estimate().budget().reset_unlimited();
        let provider = Address::generate(env);
        let mut m = Map::new(env);
        for id in 1..=n {
            m.set(id, make_signal(env, id, &provider, ts));
        }
        m
    }

    fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = Env::default();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || f(&env))
    }

    /// Small dataset: well within budget, so pagination completes in a
    /// single call (`cursor: None`) and matches the unpaginated function.
    #[test]
    fn small_dataset_completes_in_one_page() {
        with_contract(|env| {
            env.ledger().with_mut(|l| l.timestamp = 1_000_000);
            let map = make_map(env, 5, 999_000);

            let paged =
                calculate_global_analytics_paginated(env, &map, None, GlobalAnalyticsAccumulator::new());
            assert_eq!(paged.cursor, None);

            let direct = calculate_global_analytics(env, &map);
            assert_eq!(paged.accumulator.total_signals_24h, direct.total_signals_24h);
            assert_eq!(paged.accumulator.total_volume_24h, direct.total_volume_24h);
            assert_eq!(paged.accumulator.avg_success_rate(), direct.avg_success_rate);
        });
    }

    /// Large dataset: forces at least one early stop (estimated cost crosses
    /// the safety margin before the whole map is scanned), and resuming via
    /// the returned cursor across multiple calls reconciles to the same
    /// totals as a single unpaginated `calculate_global_analytics` call.
    #[test]
    fn large_dataset_paginates_and_resumes_correctly() {
        with_contract(|env| {
            env.ledger().with_mut(|l| l.timestamp = 1_000_000);
            // Budget margin (60_000_000) / est-per-signal (50_000) = 1_200
            // signals before a stop is even possible; use enough to force
            // multiple pages with the 20-item check interval.
            let n: u64 = 1_300;
            let map = make_map(env, n, 999_000);

            let mut cursor: Option<u64> = None;
            let mut acc = GlobalAnalyticsAccumulator::new();
            let mut pages = 0u32;
            loop {
                let paged = calculate_global_analytics_paginated(env, &map, cursor, acc);
                acc = paged.accumulator;
                cursor = paged.cursor;
                pages += 1;
                if cursor.is_none() {
                    break;
                }
                assert!(pages < 10_000, "pagination did not terminate");
            }
            assert!(pages > 1, "expected pagination to span multiple pages, got {pages}");

            let direct = calculate_global_analytics(env, &map);
            assert_eq!(acc.total_signals_24h, direct.total_signals_24h);
            assert_eq!(acc.total_volume_24h, direct.total_volume_24h);
            assert_eq!(acc.avg_success_rate(), direct.avg_success_rate);
        });
    }
}
