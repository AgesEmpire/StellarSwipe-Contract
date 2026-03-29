use crate::categories::SignalCategory;
use crate::types::{Signal, SignalStatus, SignalSummary, SortOption};
use soroban_sdk::{Address, Env, Map, Vec};

const MAX_LIMIT: u32 = 50;
const DEFAULT_LIMIT: u32 = 20;

/// Implement Batch Signal Querying & Feed Pagination
pub fn get_active_signals(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    provider_filter: Option<Address>,
    offset: u32,
    limit: u32,
    sort_by: SortOption,
    category_filter: Option<SignalCategory>,
) -> Vec<SignalSummary> {
    let mut active_signals = Vec::new(env);
    let current_time = env.ledger().timestamp();

    // Use category index if filter provided (requires access to contract storage)
    if let Some(category) = category_filter {
        // Note: query.rs can't access contract storage directly.
        // For now, full scan; index usage in contract call wrapper.
        // To use index, move logic or pass index_map.
    }

    // 1. Filter out expired signals and optionally filter by provider
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

    // 2. Sort the elements
    // We implement a simple bubble sort matching Soroban constraints
    for i in 0..total_active {
        for j in 0..(total_active - i - 1) {
            let curr = active_signals.get(j).unwrap();
            let next = active_signals.get(j + 1).unwrap();

            let should_swap = match sort_by {
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
                    // Tie breaker: timestamp
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
                    // Tie breaker: timestamp
                    if curr.total_volume < next.total_volume {
                        true
                    } else if curr.total_volume == next.total_volume {
                        curr.timestamp < next.timestamp
                    } else {
                        false
                    }
                }
            };

            if should_swap {
                active_signals.set(j, next);
                active_signals.set(j + 1, curr);
            }
        }
    }

    // 3. Paginate
    let mut results = Vec::new(env);
    let end = (offset + actual_limit).min(total_active);

    for i in offset..end {
        let signal = active_signals.get(i).unwrap();
        let success_rate = if signal.executions > 0 {
            (signal.successful_executions * 10000) / signal.executions
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
