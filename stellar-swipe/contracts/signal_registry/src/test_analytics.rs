#![cfg(test)]
use crate::analytics::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::types::{Signal, SignalAction, SignalStatus};
use soroban_sdk::{testutils::Address as _, Address, Env, Map, String};

fn create_test_signal(
    env: &Env,
    id: u64,
    provider: &Address,
    asset_pair: &str,
    timestamp: u64,
    executions: u32,
    total_roi: i128,
    status: SignalStatus,
) -> Signal {
    Signal {
        id,
        provider: provider.clone(),
        asset_pair: String::from_str(env, asset_pair),
        action: SignalAction::Buy,
        price: 100,
        rationale: String::from_str(env, "test"),
        timestamp,
        expiry: timestamp + 3600,
        status,
        executions,
        successful_executions: if total_roi > 0 { executions } else { 0 },
        total_volume: 1000,
        total_roi,
        category: SignalCategory::SWING,
        tags: soroban_sdk::Vec::new(env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
        submitted_at: timestamp,
        rationale_hash: String::from_str(env, "test"),
        confidence: 50,
        adoption_count: 0,
        ai_validation_score: None,
        avg_copier_roi_bps: 0,
        copier_closed_count: 0,
    }
}

#[test]
fn test_provider_analytics_insufficient_signals() {
    let env = Env::default();
    let provider = Address::generate(&env);
    let mut signals = Map::new(&env);

    // Only 5 signals (below MIN_SIGNALS_FOR_ANALYTICS = 10)
    for i in 0..5 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "XLM/USDC", 1000, 1, 500, SignalStatus::Successful),
        );
    }

    let result = calculate_provider_analytics(&env, &signals, &provider);
    assert!(result.is_none());
}

#[test]
fn test_provider_analytics_success() {
    let env = Env::default();
    let provider = Address::generate(&env);
    let mut signals = Map::new(&env);

    // 15 signals with varying performance
    for i in 0..15 {
        let roi = if i % 3 == 0 { 500 } else { 300 };
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "XLM/USDC", 1000 + i * 100, 1, roi, SignalStatus::Successful),
        );
    }

    let result = calculate_provider_analytics(&env, &signals, &provider);
    assert!(result.is_some());
    
    let analytics = result.unwrap();
    assert_eq!(analytics.total_signals, 15);
    assert!(analytics.avg_roi > 0);
}

#[test]
fn test_best_asset_pair() {
    let env = Env::default();
    let provider = Address::generate(&env);
    let mut signals = Map::new(&env);

    // XLM/USDC with high ROI
    for i in 0..5 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "XLM/USDC", 1000, 1, 1000, SignalStatus::Successful),
        );
    }

    // BTC/USDC with low ROI
    for i in 5..10 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "BTC/USDC", 1000, 1, 100, SignalStatus::Successful),
        );
    }

    let provider_signals = get_provider_signals(&signals, &provider);
    let best = find_best_asset_pair(&env, &provider_signals);
    
    assert_eq!(best, String::from_str(&env, "XLM/USDC"));
}

#[test]
fn test_win_streak() {
    let env = Env::default();
    let provider = Address::generate(&env);
    let mut signals_vec = soroban_sdk::Vec::new(&env);

    // 3 successful
    for i in 0..3 {
        signals_vec.push_back(create_test_signal(
            &env, i, &provider, "XLM/USDC", 1000, 1, 500, SignalStatus::Successful
        ));
    }

    // 1 failed (breaks streak)
    signals_vec.push_back(create_test_signal(
        &env, 3, &provider, "XLM/USDC", 1000, 1, -500, SignalStatus::Failed
    ));

    // 5 successful (new streak)
    for i in 4..9 {
        signals_vec.push_back(create_test_signal(
            &env, i, &provider, "XLM/USDC", 1000, 1, 500, SignalStatus::Successful
        ));
    }

    let streak = calculate_win_streak(&signals_vec);
    assert_eq!(streak, 5);
}

#[test]
fn test_trending_assets() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 10000);
    
    let provider = Address::generate(&env);
    let mut signals = Map::new(&env);

    // Recent signals (within 24h)
    for i in 0..10 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "XLM/USDC", 9500, 1, 500, SignalStatus::Active),
        );
    }

    for i in 10..15 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "BTC/USDC", 9500, 1, 500, SignalStatus::Active),
        );
    }

    // Old signals (outside 24h window)
    for i in 15..20 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "ETH/USDC", 1000, 1, 500, SignalStatus::Active),
        );
    }

    let trending = get_trending_assets(&env, &signals, 24);
    
    assert!(trending.len() > 0);
    let top = trending.get(0).unwrap();
    assert_eq!(top.0, String::from_str(&env, "XLM/USDC"));
    assert_eq!(top.1, 10);
}

#[test]
fn test_global_analytics() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 100000);
    
    let provider = Address::generate(&env);
    let mut signals = Map::new(&env);

    // Recent signals (within 24h)
    for i in 0..5 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "XLM/USDC", 99000, 1, 500, SignalStatus::Successful),
        );
    }

    for i in 5..8 {
        signals.set(
            i,
            create_test_signal(&env, i, &provider, "BTC/USDC", 99000, 1, -500, SignalStatus::Failed),
        );
    }

    let analytics = calculate_global_analytics(&env, &signals);
    
    assert_eq!(analytics.total_signals_24h, 8);
    assert!(analytics.avg_success_rate > 0);
    assert!(analytics.total_volume_24h > 0);
}

#[test]
fn test_avg_roi_calculation() {
    let env = Env::default();
    let provider = Address::generate(&env);
    let mut signals_vec = soroban_sdk::Vec::new(&env);

    signals_vec.push_back(create_test_signal(&env, 0, &provider, "XLM/USDC", 1000, 2, 1000, SignalStatus::Successful));
    signals_vec.push_back(create_test_signal(&env, 1, &provider, "XLM/USDC", 1000, 1, 300, SignalStatus::Successful));

    let avg = calculate_avg_roi(&signals_vec);
    assert_eq!(avg, 400); // (1000/2 + 300/1) / 2 = (500 + 300) / 2 = 400
}

#[test]
fn test_best_time_of_day() {
    let env = Env::default();
    let provider = Address::generate(&env);
    let mut signals_vec = soroban_sdk::Vec::new(&env);

    // Hour 14 (2 PM) - high ROI
    signals_vec.push_back(create_test_signal(&env, 0, &provider, "XLM/USDC", 14 * 3600, 1, 1000, SignalStatus::Successful));
    signals_vec.push_back(create_test_signal(&env, 1, &provider, "XLM/USDC", 14 * 3600 + 100, 1, 900, SignalStatus::Successful));

    // Hour 10 (10 AM) - low ROI
    signals_vec.push_back(create_test_signal(&env, 2, &provider, "XLM/USDC", 10 * 3600, 1, 100, SignalStatus::Successful));

    let best_hour = find_best_time_of_day(&signals_vec);
    assert_eq!(best_hour, 14);
}

#[test]
fn test_zero_adoption_signals_treated_as_pending() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 100000);

    let provider = Address::generate(&env);
    let mut signals = Map::new(&env);

    // Zero-adoption failed signals — should not count toward avg_success_rate
    for i in 0..5u64 {
        let mut s = create_test_signal(&env, i, &provider, "XLM/USDC", 99000, 0, 0, SignalStatus::Failed);
        s.adoption_count = 0;
        signals.set(i, s);
    }

    let analytics = calculate_global_analytics(&env, &signals);
    // No terminal signals with adoption → avg_success_rate must be 0 (no division)
    assert_eq!(analytics.avg_success_rate, 0);
    assert_eq!(analytics.total_signals_24h, 5);
}

#[test]
fn test_zero_executions_handling() {
    let env = Env::default();
    let provider = Address::generate(&env);
    let mut signals_vec = soroban_sdk::Vec::new(&env);

    // Signal with no executions
    signals_vec.push_back(create_test_signal(&env, 0, &provider, "XLM/USDC", 1000, 0, 0, SignalStatus::Active));

    let avg = calculate_avg_roi(&signals_vec);
    assert_eq!(avg, 0);
}

fn get_provider_signals(signals_map: &Map<u64, Signal>, provider: &Address) -> soroban_sdk::Vec<Signal> {
    let env = signals_map.env();
    let mut result = soroban_sdk::Vec::new(&env);
    
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

fn find_best_asset_pair(env: &Env, signals: &soroban_sdk::Vec<Signal>) -> String {
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

fn calculate_win_streak(signals: &soroban_sdk::Vec<Signal>) -> u32 {
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

fn calculate_avg_roi(signals: &soroban_sdk::Vec<Signal>) -> i128 {
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
    
    if count > 0 { total / count as i128 } else { 0 }
}

fn find_best_time_of_day(signals: &soroban_sdk::Vec<Signal>) -> u32 {
    let mut hour_roi = [0i128; 24];
    let mut hour_counts = [0u32; 24];
    
    for i in 0..signals.len() {
        let signal = signals.get(i).unwrap();
        if signal.executions > 0 {
            let hour = ((signal.timestamp % 86400) / 3600) as usize;
            if hour < 24 {
                hour_roi[hour] = hour_roi[hour].saturating_add(signal.total_roi / signal.executions as i128);
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
