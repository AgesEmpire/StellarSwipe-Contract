#![cfg(test)]

use crate::categories::{RiskLevel, SignalCategory};
use crate::churn_risk::get_provider_churn_risk;
use crate::social::SocialDataKey;
use crate::types::{Signal, SignalAction, SignalStatus};
use crate::SignalRegistry;
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env, Map, String, Vec};

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_signal(
    env: &Env,
    id: u64,
    provider: &Address,
    timestamp: u64,
    status: SignalStatus,
) -> Signal {
    Signal {
        id,
        provider: provider.clone(),
        asset_pair: String::from_str(env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100,
        rationale: String::from_str(env, "test"),
        timestamp,
        expiry: timestamp + 3600,
        status,
        executions: 1,
        successful_executions: 1,
        total_volume: 1000,
        total_roi: 500,
        category: SignalCategory::SWING,
        tags: Vec::new(env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
        adoption_count: 0,
    }
}

// ── test_churn_risk_composite ─────────────────────────────────────────────────
//
// Provider history engineered to trigger all four risk factors simultaneously:
//
//   now = 10_000_000 s
//   cutoff_30d = now - 2_592_000 = 7_408_000
//   cutoff_7d  = now -   604_800 = 9_395_200
//
//   Historical-only signals (ts ∈ [cutoff_30d, cutoff_7d)):
//     14 Successful + 5 Failed = 19 signals
//
//   Recent signal (ts = cutoff_7d + 100 = 9_395_300, inside both windows):
//     1 Failed
//
//   Total counts:
//     historical_total    = 20  (all 20 are >= cutoff_30d)
//     historical_terminal = 20  (14 Successful + 5 Failed + 1 recent Failed)
//     historical_success  = 14
//     historical_rate     = 14 * 100 / 20 = 70
//     recent_total        = 1
//     recent_terminal     = 1,  recent_success = 0,  recent_rate = 0
//     last_signal_ts      = 9_395_300
//     idle_secs           = 10_000_000 - 9_395_300 = 604_700
//
//   Expected sub-scores:
//     frequency_drop:      expected_recent = 20*7/30 = 4
//                          drop = 4 - 1 = 3
//                          score = 3 * 25 / 4 = 18
//     follower_loss:       peak = 100, current = 40
//                          loss = 60, score = 60 * 25 / 100 = 15
//     performance_decline: historical_rate = 70, recent_rate = 0
//                          decline = 70, score = 70 * 25 / 70 = 25
//     inactivity:          idle = 604_700, max = 14*86400 = 1_209_600
//                          score = 604_700 * 25 / 1_209_600 = 12  (floor)
//     composite            = 18 + 15 + 25 + 12 = 70

#[test]
fn test_churn_risk_composite() {
    let env = Env::default();
    let now: u64 = 10_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);

    // Register SignalRegistry contract so instance storage is accessible.
    let contract_id = env.register(SignalRegistry, ());

    let provider = Address::generate(&env);
    let mut signals: Map<u64, Signal> = Map::new(&env);

    let cutoff_30d = now - 30 * 24 * 3600; // 7_408_000
    let cutoff_7d  = now -  7 * 24 * 3600; // 9_395_200

    // 14 Successful historical-only signals (inside 30d, outside 7d)
    let mut id = 0u64;
    for i in 0..14u64 {
        let ts = cutoff_30d + 100_000 + i * 10_000;
        signals.set(id, make_signal(&env, id, &provider, ts, SignalStatus::Successful));
        id += 1;
    }
    // 5 Failed historical-only signals
    for i in 0..5u64 {
        let ts = cutoff_30d + 200_000 + i * 10_000;
        signals.set(id, make_signal(&env, id, &provider, ts, SignalStatus::Failed));
        id += 1;
    }
    // 1 recent Failed signal (inside 7d window, also inside 30d window)
    let recent_ts = cutoff_7d + 100;
    signals.set(id, make_signal(&env, id, &provider, recent_ts, SignalStatus::Failed));

    let peak_followers: u32 = 100;

    // Run the churn risk computation inside a contract context so that
    // instance storage (follower count) is accessible.
    let score = env.as_contract(&contract_id, || {
        // Seed follower count: current = 40, peak = 100.
        env.storage()
            .instance()
            .set(&SocialDataKey::FollowerCount(provider.clone()), &40u32);

        get_provider_churn_risk(&env, &signals, &provider, peak_followers)
    });

    // ── Verify each sub-score is non-zero ────────────────────────────────────
    assert!(
        score.frequency_drop_score > 0,
        "frequency_drop_score must be non-zero, got {}",
        score.frequency_drop_score
    );
    assert!(
        score.follower_loss_score > 0,
        "follower_loss_score must be non-zero, got {}",
        score.follower_loss_score
    );
    assert!(
        score.performance_decline_score > 0,
        "performance_decline_score must be non-zero, got {}",
        score.performance_decline_score
    );
    assert!(
        score.inactivity_score > 0,
        "inactivity_score must be non-zero, got {}",
        score.inactivity_score
    );

    // ── Verify composite equals sum of sub-scores ─────────────────────────────
    let expected_composite = score.frequency_drop_score
        + score.follower_loss_score
        + score.performance_decline_score
        + score.inactivity_score;
    assert_eq!(
        score.composite_score, expected_composite,
        "composite_score must equal sum of sub-scores"
    );

    // ── Verify concrete expected values ──────────────────────────────────────
    // historical_total=20, expected_recent = 20*7/30 = 4, actual=1, drop=3
    // frequency_drop = 3*25/4 = 18
    assert_eq!(score.frequency_drop_score, 18, "frequency_drop_score");

    // follower_loss = (100-40)*25/100 = 15
    assert_eq!(score.follower_loss_score, 15, "follower_loss_score");

    // historical_rate = 14*100/20 = 70, recent_rate = 0
    // performance_decline = 70*25/70 = 25
    assert_eq!(score.performance_decline_score, 25, "performance_decline_score");

    // idle = 604_700 s, max = 1_209_600 s
    // inactivity = 604_700*25/1_209_600 = 12
    assert_eq!(score.inactivity_score, 12, "inactivity_score");

    assert_eq!(score.composite_score, 70, "composite_score");
}

// ── test_churn_risk_no_activity ───────────────────────────────────────────────
//
// Brand-new provider: zero signals, zero followers, zero peak.
// Must not panic and must return a defined default — all zeros.

#[test]
fn test_churn_risk_no_activity() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000_000);

    let contract_id = env.register(SignalRegistry, ());
    let provider = Address::generate(&env);
    let signals: Map<u64, Signal> = Map::new(&env);

    let score = env.as_contract(&contract_id, || {
        get_provider_churn_risk(&env, &signals, &provider, 0)
    });

    assert_eq!(score.frequency_drop_score, 0, "no-activity: frequency_drop must be 0");
    assert_eq!(score.follower_loss_score, 0, "no-activity: follower_loss must be 0");
    assert_eq!(score.performance_decline_score, 0, "no-activity: performance_decline must be 0");
    assert_eq!(score.inactivity_score, 0, "no-activity: inactivity must be 0 (no signals ever)");
    assert_eq!(score.composite_score, 0, "no-activity: composite must be 0");
}
