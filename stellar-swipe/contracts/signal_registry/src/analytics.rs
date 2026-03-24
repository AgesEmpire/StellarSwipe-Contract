use crate::social::get_follower_count;
use crate::types::{Signal, SignalStatus};
use soroban_sdk::{Address, Env, Map, String, Vec};

const MIN_SIGNALS_FOR_ANALYTICS: u32 = 10;
const HOURS_24: u64 = 86400;

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
    let cutoff = env.ledger().timestamp().saturating_sub(window_hours * 3600);
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
    let cutoff = env.ledger().timestamp().saturating_sub(HOURS_24);
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
                ) {
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
