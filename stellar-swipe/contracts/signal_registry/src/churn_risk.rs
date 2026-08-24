use crate::social::get_follower_count;
use crate::types::{Signal, SignalStatus};
use soroban_sdk::{contracttype, Address, Env, Map};

/// Maximum score per factor (4 factors × 25 = 100 total).
const FACTOR_MAX: u32 = 25;
/// Seconds in 7 days — used as the "recent" window for frequency/performance.
const WINDOW_7D: u64 = 7 * 24 * 3600;
/// Seconds in 30 days — used as the "historical" window.
const WINDOW_30D: u64 = 30 * 24 * 3600;
/// Inactivity threshold: 14 days with no new signal → max inactivity score.
const INACTIVITY_MAX_SECS: u64 = 14 * 24 * 3600;

#[contracttype]
#[derive(Clone, Debug)]
pub struct ChurnRiskScore {
    /// 0–25: high value = large drop in signal frequency.
    pub frequency_drop_score: u32,
    /// 0–25: high value = significant follower loss.
    pub follower_loss_score: u32,
    /// 0–25: high value = recent success rate well below historical.
    pub performance_decline_score: u32,
    /// 0–25: high value = long inactivity window.
    pub inactivity_score: u32,
    /// Sum of all four sub-scores (0–100).
    pub composite_score: u32,
}

/// Compute churn risk for `provider` given the full signals map.
///
/// Returns a `ChurnRiskScore` with each sub-score and the composite.
/// A brand-new provider with zero signals returns all zeros (no panic).
pub fn get_provider_churn_risk(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    provider: &Address,
    peak_followers: u32,
) -> ChurnRiskScore {
    let now = env.ledger().timestamp();

    // Collect provider signals once.
    let mut recent_total = 0u32;
    let mut historical_total = 0u32;
    let mut recent_success = 0u32;
    let mut recent_terminal = 0u32;
    let mut historical_success = 0u32;
    let mut historical_terminal = 0u32;
    let mut last_signal_ts: u64 = 0;

    let cutoff_recent = now.saturating_sub(WINDOW_7D);
    let cutoff_historical = now.saturating_sub(WINDOW_30D);

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.provider != *provider {
                    continue;
                }
                if signal.timestamp > last_signal_ts {
                    last_signal_ts = signal.timestamp;
                }
                if signal.timestamp >= cutoff_historical {
                    historical_total += 1;
                    if matches!(signal.status, SignalStatus::Successful | SignalStatus::Failed) {
                        historical_terminal += 1;
                        if signal.status == SignalStatus::Successful {
                            historical_success += 1;
                        }
                    }
                }
                if signal.timestamp >= cutoff_recent {
                    recent_total += 1;
                    if matches!(signal.status, SignalStatus::Successful | SignalStatus::Failed) {
                        recent_terminal += 1;
                        if signal.status == SignalStatus::Successful {
                            recent_success += 1;
                        }
                    }
                }
            }
        }
    }

    // ── Factor 1: Signal frequency drop ──────────────────────────────────────
    // Expected recent signals = historical_total * (7/30).
    // Drop ratio = max(0, expected - recent) / expected, scaled to FACTOR_MAX.
    let frequency_drop_score = if historical_total == 0 {
        0
    } else {
        // expected_recent ≈ historical_total * 7 / 30 (integer, floor)
        let expected_recent = (historical_total as u64 * 7 / 30) as u32;
        if expected_recent == 0 || recent_total >= expected_recent {
            0
        } else {
            let drop = expected_recent - recent_total;
            // score = (drop / expected_recent) * FACTOR_MAX
            (drop * FACTOR_MAX / expected_recent).min(FACTOR_MAX)
        }
    };

    // ── Factor 2: Follower loss ───────────────────────────────────────────────
    let current_followers = get_follower_count(env, provider);
    let follower_loss_score = if peak_followers == 0 {
        0
    } else if current_followers >= peak_followers {
        0
    } else {
        let loss = peak_followers - current_followers;
        (loss * FACTOR_MAX / peak_followers).min(FACTOR_MAX)
    };

    // ── Factor 3: Performance decline ────────────────────────────────────────
    // Compare recent success rate vs historical success rate.
    let historical_rate = if historical_terminal > 0 {
        historical_success * 100 / historical_terminal
    } else {
        0
    };
    let recent_rate = if recent_terminal > 0 {
        recent_success * 100 / recent_terminal
    } else if recent_total > 0 {
        // Signals exist but none terminal yet — treat as neutral (no decline).
        historical_rate
    } else {
        // No recent signals at all — treat as 0% recent rate if there's history.
        if historical_terminal > 0 { 0 } else { historical_rate }
    };

    let performance_decline_score = if historical_rate == 0 {
        0
    } else if recent_rate >= historical_rate {
        0
    } else {
        let decline = historical_rate - recent_rate;
        (decline * FACTOR_MAX / historical_rate).min(FACTOR_MAX)
    };

    // ── Factor 4: Inactivity window ───────────────────────────────────────────
    let inactivity_score = if last_signal_ts == 0 {
        // No signals ever — brand-new provider, return 0 (not punished).
        0
    } else {
        let idle_secs = now.saturating_sub(last_signal_ts);
        if idle_secs >= INACTIVITY_MAX_SECS {
            FACTOR_MAX
        } else {
            ((idle_secs * FACTOR_MAX as u64) / INACTIVITY_MAX_SECS) as u32
        }
    };

    let composite_score = frequency_drop_score
        + follower_loss_score
        + performance_decline_score
        + inactivity_score;

    ChurnRiskScore {
        frequency_drop_score,
        follower_loss_score,
        performance_decline_score,
        inactivity_score,
        composite_score,
    }
}
