#![cfg(test)]

//! Churn-risk composite-score integration tests (Issue #944).
//!
//! `get_provider_churn_risk` aggregates three weighted components — signal
//! frequency decline (40 %), follower loss (30 %), and performance decline
//! (30 %) — into a single 0–100 score. These tests exercise all components
//! simultaneously with known inputs so that an error in the weighted
//! aggregation (e.g. integer-division truncation when multiple factors are
//! non-zero) is caught, and verify that a brand-new provider with zero
//! activity returns a defined default score without panicking.

use crate::categories::{RiskLevel, SignalCategory};
use crate::churn_risk::{
    get_provider_churn_risk, ChurnRiskLevel, ChurnStorageKey, ProviderChurnSnapshot,
};
use crate::social::SocialDataKey;
use crate::types::{ProviderPerformance, Signal, SignalAction, SignalStatus};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Map, String, Vec};

/// 30-day sliding window in seconds (mirrors `churn_risk::WINDOW_SECS`).
const WINDOW_SECS: u64 = 30 * 24 * 3_600;

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

fn perf(success_rate: u32) -> ProviderPerformance {
    ProviderPerformance {
        success_rate,
        ..Default::default()
    }
}

/// Persistent-storage access requires an active contract context in
/// soroban-sdk 23.x, so tests register the real contract and run inside it.
fn with_contract<R>(env: &Env, f: impl FnOnce() -> R) -> R {
    #[allow(deprecated)]
    let cid = env.register_contract(None, crate::SignalRegistry);
    env.as_contract(&cid, f)
}

// ─── All risk factors active simultaneously ─────────────────────────────────

#[test]
fn test_churn_risk_composite() {
    let env = make_env();
    let provider = Address::generate(&env);
    let now = env.ledger().timestamp();
    let recent_ts = now.saturating_sub(WINDOW_SECS / 2); // inside recent window
    let prior_ts = now.saturating_sub(WINDOW_SECS + 100); // prior window, outside recent

    with_contract(&env, || {
        let mut signals: Map<u64, Signal> = Map::new(&env);

        // Recent window: 3 closed signals, 1 successful + 2 failed
        // → recent_rate = 1 * 10_000 / 3 = 3333 bps.
        for i in 0..1u64 {
            signals.set(i, base_signal(&env, i, &provider, recent_ts));
        }
        for i in 1..3u64 {
            let mut s = base_signal(&env, i, &provider, recent_ts);
            s.status = SignalStatus::Failed;
            signals.set(i, s);
        }
        // Prior window: 8 successful signals → strong historical activity.
        for i in 3..11u64 {
            signals.set(i, base_signal(&env, i, &provider, prior_ts));
        }

        // Follower-loss factor: snapshot baseline 100 followers vs 40 now.
        env.storage().persistent().set(
            &ChurnStorageKey::ProviderSnapshot(provider.clone()),
            &ProviderChurnSnapshot {
                follower_count: 100,
                signal_count: 11,
                success_rate_bps: 7_500,
                snapshot_at: now.saturating_sub(WINDOW_SECS),
            },
        );
        env.storage()
            .instance()
            .set(&SocialDataKey::FollowerCount(provider.clone()), &40u32);

        // Performance-decline factor: all-time rate 7500 bps > recent 3333 bps.
        let stats = perf(7_500);
        let score = get_provider_churn_risk(&env, &provider, &signals, Some(&stats));

        // Expected component scores (integer division, truncated):
        //   freq    = (8 - 3) * 100 / 8           = 62
        //   follower= (100 - 40) * 100 / 100      = 60
        //   perf    = (7500 - 3333) * 100 / 7500  = 55
        assert_eq!(score.signal_freq_score, 62, "signal freq component");
        assert_eq!(score.follower_unsub_score, 60, "follower component");
        assert_eq!(score.perf_trend_score, 55, "performance component");

        // Every component contributes a non-zero amount to the composite.
        assert!(score.signal_freq_score > 0);
        assert!(score.follower_unsub_score > 0);
        assert!(score.perf_trend_score > 0);

        // Composite = (62*40 + 60*30 + 55*30) / 100 = 5930 / 100 = 59.
        assert_eq!(score.composite_score, 59, "weighted composite");
        assert_eq!(score.level, ChurnRiskLevel::Medium);
        assert_eq!(score.computed_at, now);
    });
}

// ─── Brand-new provider with zero activity ──────────────────────────────────

#[test]
fn test_churn_risk_no_activity() {
    let env = make_env();
    let provider = Address::generate(&env);
    let now = env.ledger().timestamp();

    with_contract(&env, || {
        // No signals, no snapshot, no stats → must not panic and must return a
        // defined default score.
        let signals: Map<u64, Signal> = Map::new(&env);
        let score = get_provider_churn_risk(&env, &provider, &signals, None);

        //   freq     = 50 (no prior activity, no recent activity)
        //   follower = 0 (no snapshot to compare against)
        //   perf     = 0 (no stats)
        //   composite= 50 * 40 / 100 = 20 → Low
        assert_eq!(score.signal_freq_score, 50, "default freq component");
        assert_eq!(score.follower_unsub_score, 0, "default follower component");
        assert_eq!(score.perf_trend_score, 0, "default performance component");
        assert_eq!(score.composite_score, 20, "default composite score");
        assert_eq!(score.level, ChurnRiskLevel::Low);
        assert_eq!(score.computed_at, now);
    });
}
