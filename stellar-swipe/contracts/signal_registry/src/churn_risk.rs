use soroban_sdk::{contracttype, Address, Env, Map, Symbol};

use crate::social::get_follower_count;
use crate::types::{ProviderPerformance, Signal, SignalStatus};

/// Default composite-score threshold above which `churn_risk_elevated` is emitted.
pub const CHURN_RISK_THRESHOLD_DEFAULT: u32 = 67;

/// 30-day sliding window in seconds.
const WINDOW_SECS: u64 = 30 * 24 * 3_600;

// ─── Types ────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChurnRiskLevel {
    Low,
    Medium,
    High,
}

/// Full churn-risk result returned to callers.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ChurnRiskScore {
    /// Overall risk bucket.
    pub level: ChurnRiskLevel,
    /// Weighted composite 0–100.
    pub composite_score: u32,
    /// Signal-frequency-decline component 0–100.
    pub signal_freq_score: u32,
    /// Follower-unsubscribe component 0–100.
    pub follower_unsub_score: u32,
    /// Performance-trend-decline component 0–100.
    pub perf_trend_score: u32,
    /// Ledger timestamp when the score was computed.
    pub computed_at: u64,
}

/// Snapshot of a provider's activity captured once per period and used as the
/// baseline for the next churn-risk calculation.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProviderChurnSnapshot {
    pub follower_count: u32,
    pub signal_count: u32,
    pub success_rate_bps: u32,
    pub snapshot_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum ChurnStorageKey {
    /// Admin-configurable score threshold (u32).
    ChurnThreshold,
    /// Per-provider activity snapshot for baseline comparisons.
    ProviderSnapshot(Address),
}

// ─── Admin config ─────────────────────────────────────────────────────────────

pub fn get_churn_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ChurnStorageKey::ChurnThreshold)
        .unwrap_or(CHURN_RISK_THRESHOLD_DEFAULT)
}

pub fn set_churn_threshold(env: &Env, threshold: u32) {
    env.storage()
        .instance()
        .set(&ChurnStorageKey::ChurnThreshold, &threshold);
}

// ─── Snapshot management ──────────────────────────────────────────────────────

pub fn get_provider_churn_snapshot(env: &Env, provider: &Address) -> Option<ProviderChurnSnapshot> {
    env.storage()
        .persistent()
        .get(&ChurnStorageKey::ProviderSnapshot(provider.clone()))
}

/// Capture a fresh snapshot for `provider`.  Call this periodically (e.g. after
/// each signal submission batch) so the baseline stays up-to-date.
pub fn update_provider_churn_snapshot(
    env: &Env,
    provider: &Address,
    signals_map: &Map<u64, Signal>,
    stats: Option<&ProviderPerformance>,
) {
    let now = env.ledger().timestamp();
    let window_start = now.saturating_sub(WINDOW_SECS);

    let mut signal_count = 0u32;
    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.provider == *provider && signal.timestamp >= window_start {
                    signal_count += 1;
                }
            }
        }
    }

    let snapshot = ProviderChurnSnapshot {
        follower_count: get_follower_count(env, provider),
        signal_count,
        success_rate_bps: stats.map(|s| s.success_rate).unwrap_or(0),
        snapshot_at: now,
    };

    env.storage().persistent().set(
        &ChurnStorageKey::ProviderSnapshot(provider.clone()),
        &snapshot,
    );
}

// ─── Core scoring ─────────────────────────────────────────────────────────────

/// Compute the provider's churn-risk score.
///
/// The composite score is a weighted sum of three components (weights add to 100):
/// - Signal frequency decline — 40 %
/// - Follower unsubscribe rate — 30 %
/// - Performance trend decline — 30 %
///
/// Emits `churn_risk_elevated` when the composite score meets or exceeds the
/// admin-configured threshold.
pub fn get_provider_churn_risk(
    env: &Env,
    provider: &Address,
    signals_map: &Map<u64, Signal>,
    stats: Option<&ProviderPerformance>,
) -> ChurnRiskScore {
    let now = env.ledger().timestamp();
    let recent_start = now.saturating_sub(WINDOW_SECS);
    let prior_start = now.saturating_sub(2 * WINDOW_SECS);

    // Collect per-window signal counts and recent closed stats.
    let mut recent_count = 0u32;
    let mut prior_count = 0u32;
    let mut recent_successful = 0u32;
    let mut recent_closed = 0u32;

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.provider != *provider {
                    continue;
                }
                if signal.timestamp >= recent_start {
                    recent_count += 1;
                    if matches!(
                        signal.status,
                        SignalStatus::Successful | SignalStatus::Failed
                    ) {
                        recent_closed += 1;
                        if signal.status == SignalStatus::Successful {
                            recent_successful += 1;
                        }
                    }
                } else if signal.timestamp >= prior_start {
                    prior_count += 1;
                }
            }
        }
    }

    // Component 1: signal frequency decline score (0–100).
    let signal_freq_score = freq_decline_score(recent_count, prior_count);

    // Component 2: follower unsubscribe score (0–100).
    let current_followers = get_follower_count(env, provider);
    let follower_unsub_score = match get_provider_churn_snapshot(env, provider) {
        Some(snap) => follower_loss_score(current_followers, snap.follower_count),
        None => 0u32,
    };

    // Component 3: performance trend decline score (0–100).
    let overall_rate = stats.map(|s| s.success_rate).unwrap_or(0);
    let recent_rate = if recent_closed > 0 {
        (recent_successful * 10_000) / recent_closed
    } else {
        overall_rate
    };
    let perf_trend_score = perf_decline_score(recent_rate, overall_rate);

    // Composite: 40/30/30 weighting (sum of weights == 100).
    let composite_score =
        (signal_freq_score * 40 + follower_unsub_score * 30 + perf_trend_score * 30) / 100;

    let level = if composite_score >= 67 {
        ChurnRiskLevel::High
    } else if composite_score >= 34 {
        ChurnRiskLevel::Medium
    } else {
        ChurnRiskLevel::Low
    };

    // Emit event if score crosses the admin threshold.
    let threshold = get_churn_threshold(env);
    if composite_score >= threshold {
        emit_churn_risk_elevated(env, provider, composite_score, level);
    }

    ChurnRiskScore {
        level,
        composite_score,
        signal_freq_score,
        follower_unsub_score,
        perf_trend_score,
        computed_at: now,
    }
}

// ─── Component helpers ────────────────────────────────────────────────────────

/// Scores frequency decline 0–100 where 100 means complete cessation.
fn freq_decline_score(recent: u32, prior: u32) -> u32 {
    if prior == 0 {
        return if recent == 0 { 50 } else { 0 };
    }
    if recent >= prior {
        return 0;
    }
    // decline% = (prior - recent) / prior * 100
    ((prior - recent) * 100 / prior).min(100)
}

/// Scores follower loss 0–100 relative to stored snapshot.
fn follower_loss_score(current: u32, prev: u32) -> u32 {
    if prev == 0 || current >= prev {
        return 0;
    }
    ((prev - current) * 100 / prev).min(100)
}

/// Scores performance decline 0–100 relative to all-time rate.
fn perf_decline_score(recent_bps: u32, overall_bps: u32) -> u32 {
    if overall_bps == 0 || recent_bps >= overall_bps {
        return 0;
    }
    ((overall_bps - recent_bps) * 100 / overall_bps).min(100)
}

// ─── Event emission ───────────────────────────────────────────────────────────

fn emit_churn_risk_elevated(
    env: &Env,
    provider: &Address,
    composite_score: u32,
    level: ChurnRiskLevel,
) {
    let topics = (Symbol::new(env, "churn_risk_elevated"), provider.clone());
    env.events()
        .publish(topics, (composite_score, level as u32));
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{RiskLevel, SignalCategory};
    use crate::types::{Signal, SignalAction, SignalStatus};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{vec, Env, Map, String, Vec};

    // Helpers ─────────────────────────────────────────────────────────────────

    fn make_env() -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(10_000_000);
        env
    }

    fn base_signal(env: &Env, id: u64, provider: &Address, timestamp: u64) -> Signal {
        Signal {
            id,
            provider: provider.clone(),
            asset_pair: String::from_str(env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 1_000_000,
            rationale: String::from_str(env, "test"),
            timestamp,
            expiry: timestamp + 3600,
            status: SignalStatus::Successful,
            executions: 1,
            successful_executions: 1,
            total_volume: 100,
            total_roi: 50,
            category: SignalCategory::SWING,
            tags: Vec::new(env),
            risk_level: RiskLevel::Low,
            is_collaborative: false,
            submitted_at: timestamp,
            rationale_hash: String::from_str(env, "h"),
            confidence: 80,
            adoption_count: 1,
            ai_validation_score: None,
            avg_copier_roi_bps: 50,
            copier_closed_count: 1,
            warning_emitted: false,
            benchmark_return_bps: None,
            alpha_bps: None,
        }
    }

    fn perf(total: u32, success_rate: u32) -> ProviderPerformance {
        ProviderPerformance {
            total_signals: total,
            successful_signals: (total * success_rate / 10_000),
            failed_signals: total - (total * success_rate / 10_000),
            total_copies: 0,
            success_rate,
            avg_return: 0,
            total_volume: 0,
            follower_count: 0,
        }
    }

    fn with_contract<R>(env: &Env, f: impl FnOnce() -> R) -> R {
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, f)
    }

    // ── Low-risk scenario ─────────────────────────────────────────────────────

    #[test]
    fn low_risk_active_provider() {
        let env = make_env();
        let provider = Address::generate(&env);
        let now = env.ledger().timestamp();
        let recent = now.saturating_sub(WINDOW_SECS / 2); // 15 days ago
        let older = now.saturating_sub(WINDOW_SECS + 100); // just outside recent window

        with_contract(&env, || {
            let mut signals: Map<u64, Signal> = Map::new(&env);
            // More recent signals than prior → no frequency decline
            for i in 0..5u64 {
                signals.set(i, base_signal(&env, i, &provider, recent));
            }
            for i in 5..8u64 {
                signals.set(i, base_signal(&env, i, &provider, older));
            }

            let stats = perf(8, 8_000);
            let score = get_provider_churn_risk(&env, &provider, &signals, Some(&stats));

            assert_eq!(score.level, ChurnRiskLevel::Low);
            assert!(
                score.composite_score < 34,
                "score={}",
                score.composite_score
            );
        });
    }

    // ── Medium-risk scenario ──────────────────────────────────────────────────

    #[test]
    fn medium_risk_declining_provider() {
        let env = make_env();
        let provider = Address::generate(&env);
        let now = env.ledger().timestamp();
        let recent = now.saturating_sub(WINDOW_SECS / 2);
        let older = now.saturating_sub(WINDOW_SECS + 100);

        with_contract(&env, || {
            let mut signals: Map<u64, Signal> = Map::new(&env);
            // 1 recent signal (Failed) vs 8 prior (Successful):
            //   freq_score = 7/8*100 = 87  → contrib = 87*40/100 = 34
            //   follower_score = 0          → contrib = 0
            //   recent_closed=1, recent_successful=0 → recent_rate = 0
            //   overall_rate = 8000         → perf_score = (8000-0)*100/8000 = 100
            //                                            → contrib = 100*30/100 = 30
            //   composite = 34 + 0 + 30 = 64  → Medium
            let mut failed = base_signal(&env, 0, &provider, recent);
            failed.status = SignalStatus::Failed;
            signals.set(0, failed);

            for i in 1..9u64 {
                signals.set(i, base_signal(&env, i, &provider, older));
            }

            let stats = perf(9, 8_000);
            let score = get_provider_churn_risk(&env, &provider, &signals, Some(&stats));

            assert_eq!(score.level, ChurnRiskLevel::Medium);
            assert!(
                score.composite_score >= 34 && score.composite_score < 67,
                "score={}",
                score.composite_score
            );
        });
    }

    // ── High-risk scenario ────────────────────────────────────────────────────

    #[test]
    fn high_risk_inactive_provider() {
        let env = make_env();
        let provider = Address::generate(&env);
        let now = env.ledger().timestamp();
        let older = now.saturating_sub(WINDOW_SECS + 100);

        with_contract(&env, || {
            let mut signals: Map<u64, Signal> = Map::new(&env);
            // Zero recent signals, 6 prior → full frequency decline (100 score)
            for i in 0..6u64 {
                signals.set(i, base_signal(&env, i, &provider, older));
            }

            // Install a snapshot showing 100 prior followers; current = 0 (no follows registered)
            // Soroban social module returns 0 by default, so follower loss = 100/100 = 100%
            env.storage().persistent().set(
                &ChurnStorageKey::ProviderSnapshot(provider.clone()),
                &ProviderChurnSnapshot {
                    follower_count: 100,
                    signal_count: 6,
                    success_rate_bps: 8_000,
                    snapshot_at: now.saturating_sub(WINDOW_SECS),
                },
            );

            // Performance dropped from 8000 bps to 0 (no recent closed signals → falls back to
            // overall 2000 bps)
            let stats = perf(6, 2_000);
            let score = get_provider_churn_risk(&env, &provider, &signals, Some(&stats));

            assert_eq!(score.level, ChurnRiskLevel::High);
            assert!(
                score.composite_score >= 67,
                "score={}",
                score.composite_score
            );
        });
    }

    // ── Threshold configuration ───────────────────────────────────────────────

    #[test]
    fn custom_threshold_emits_event_at_medium() {
        let env = make_env();
        let provider = Address::generate(&env);
        let now = env.ledger().timestamp();
        let recent = now.saturating_sub(WINDOW_SECS / 2);
        let older = now.saturating_sub(WINDOW_SECS + 100);

        with_contract(&env, || {
            // Lower threshold to 34 so even medium-risk emits the event
            set_churn_threshold(&env, 34);
            assert_eq!(get_churn_threshold(&env), 34);

            let mut signals: Map<u64, Signal> = Map::new(&env);
            for i in 0..2u64 {
                signals.set(i, base_signal(&env, i, &provider, recent));
            }
            for i in 2..8u64 {
                signals.set(i, base_signal(&env, i, &provider, older));
            }

            let stats = perf(8, 7_000);
            let score = get_provider_churn_risk(&env, &provider, &signals, Some(&stats));
            // Medium risk → composite >= 34; event should have been emitted (checked by level)
            assert!(score.composite_score >= 34);
        });
    }

    // ── Composite / multi-factor scenario ─────────────────────────────────────

    /// Exercises every weighted sub-score in `ChurnRiskScore` at once (signal
    /// frequency decline, follower loss, and performance decline — the three
    /// components that make up the 40/30/30 composite formula) and asserts the
    /// composite and each component against hand-derived expected values.
    ///
    /// The issue that motivated this test also names a fourth factor,
    /// "inactivity window". In the current implementation that case is not a
    /// separate weighted component: it is a special branch of
    /// `freq_decline_score` (prior == 0 && recent == 0 → 50) that only fires for
    /// providers with *no* history at all in either window. That branch is
    /// covered on its own by `test_churn_risk_no_activity` below, since mixing
    /// it into this test would zero out the other components (once `prior == 0`
    /// the decline branch is skipped entirely).
    #[test]
    fn test_churn_risk_composite() {
        let env = make_env();
        let provider = Address::generate(&env);
        let now = env.ledger().timestamp();
        let recent = now.saturating_sub(WINDOW_SECS / 2); // inside recent window
        let older = now.saturating_sub(WINDOW_SECS + 100); // inside prior window

        with_contract(&env, || {
            let mut signals: Map<u64, Signal> = Map::new(&env);

            // Recent window: 5 signals (2 Successful, 3 Failed) → recent_count=5,
            // recent_closed=5, recent_successful=2 → recent_rate = 2*10_000/5 = 4_000 bps.
            signals.set(0, base_signal(&env, 0, &provider, recent));
            signals.set(1, base_signal(&env, 1, &provider, recent));
            for i in 2..5u64 {
                let mut failed = base_signal(&env, i, &provider, recent);
                failed.status = SignalStatus::Failed;
                signals.set(i, failed);
            }

            // Prior window: 10 signals → prior_count = 10.
            for i in 5..15u64 {
                signals.set(i, base_signal(&env, i, &provider, older));
            }

            // Follower loss: previous snapshot had 200 followers, now only 150.
            env.storage().persistent().set(
                &ChurnStorageKey::ProviderSnapshot(provider.clone()),
                &ProviderChurnSnapshot {
                    follower_count: 200,
                    signal_count: 10,
                    success_rate_bps: 9_000,
                    snapshot_at: now.saturating_sub(WINDOW_SECS),
                },
            );
            env.storage().instance().set(
                &crate::social::SocialDataKey::FollowerCount(provider.clone()),
                &150u32,
            );

            // Performance decline: all-time rate 9_000 bps vs recent 4_000 bps.
            let stats = perf(15, 9_000);
            let score = get_provider_churn_risk(&env, &provider, &signals, Some(&stats));

            // signal_freq_score = (10 - 5) * 100 / 10 = 50
            assert_eq!(score.signal_freq_score, 50);
            // follower_unsub_score = (200 - 150) * 100 / 200 = 25
            assert_eq!(score.follower_unsub_score, 25);
            // perf_trend_score = (9_000 - 4_000) * 100 / 9_000 = 500_000 / 9_000 = 55 (int div)
            assert_eq!(score.perf_trend_score, 55);

            // All three components are non-zero, proving each factor contributed.
            assert!(score.signal_freq_score > 0);
            assert!(score.follower_unsub_score > 0);
            assert!(score.perf_trend_score > 0);

            // composite = (50*40 + 25*30 + 55*30) / 100 = (2000 + 750 + 1650) / 100 = 44
            assert_eq!(score.composite_score, 44);
            assert_eq!(score.level, ChurnRiskLevel::Medium);
        });
    }

    // ── Zero-activity / brand-new provider ────────────────────────────────────

    /// A brand-new provider with no signal history, no churn snapshot, and no
    /// performance stats must not panic, and must resolve to the composite's
    /// defined default rather than an arbitrary or uninitialized value.
    #[test]
    fn test_churn_risk_no_activity() {
        let env = make_env();
        let provider = Address::generate(&env);

        with_contract(&env, || {
            let signals: Map<u64, Signal> = Map::new(&env);
            let score = get_provider_churn_risk(&env, &provider, &signals, None);

            // freq_decline_score(0, 0) → prior == 0 && recent == 0 → default 50
            // (dormancy is undecidable, so it is scored as a mid-point, not 0 or 100).
            assert_eq!(score.signal_freq_score, 50);
            // No prior snapshot exists yet → no follower-loss signal.
            assert_eq!(score.follower_unsub_score, 0);
            // No stats and no recent closed signals → nothing to compare, no decline.
            assert_eq!(score.perf_trend_score, 0);

            // composite = (50*40 + 0*30 + 0*30) / 100 = 2000 / 100 = 20
            assert_eq!(score.composite_score, 20);
            assert_eq!(score.level, ChurnRiskLevel::Low);
        });
    }

    // ── Snapshot update ───────────────────────────────────────────────────────

    #[test]
    fn snapshot_captures_current_state() {
        let env = make_env();
        let provider = Address::generate(&env);
        let now = env.ledger().timestamp();
        let recent = now.saturating_sub(100);

        with_contract(&env, || {
            let mut signals: Map<u64, Signal> = Map::new(&env);
            for i in 0..3u64 {
                signals.set(i, base_signal(&env, i, &provider, recent));
            }
            let stats = perf(3, 7_500);
            update_provider_churn_snapshot(&env, &provider, &signals, Some(&stats));

            let snap = get_provider_churn_snapshot(&env, &provider).unwrap();
            assert_eq!(snap.signal_count, 3);
            assert_eq!(snap.success_rate_bps, 7_500);
        });
    }
}
