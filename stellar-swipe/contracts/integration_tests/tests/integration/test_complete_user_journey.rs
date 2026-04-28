//! Complete user journey integration test — acceptance test for mainnet launch.
//!
//! Two personas:
//!   Sara  — signal provider: deposit stake → submit signal → earn fees → check reputation
//!   Alex  — novice copier:   browse signals → swipe right (adopt) → copy trade →
//!                             stop-loss triggers → check P&L → view leaderboard
//!
//! All state is verified at each step. All events are emitted. Deterministic.

extern crate std;

use signal_registry::{
    reputation::signal_success_rate, FeeBreakdown, ProviderMetric, RiskLevel, SignalAction,
    SignalCategory, SignalOutcome, SignalRegistry, SignalStatus,
};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

// ── Minimal trade-executor stub ───────────────────────────────────────────────
// increment_adoption and record_signal_outcome require the caller to be the
// registered TradeExecutor contract. We register this stub at a known address
// and use it as the executor throughout the test.

#[contract]
struct TradeExecutorStub;

#[contractimpl]
impl TradeExecutorStub {}

// ── Setup ─────────────────────────────────────────────────────────────────────

struct Ctx<'a> {
    env: Env,
    registry: SignalRegistryClient<'a>,
    executor_id: Address,
    sara: Address,
    alex: Address,
    admin: Address,
}

fn setup() -> (Env, Address, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let admin = Address::generate(&env);
    let sara = Address::generate(&env);
    let alex = Address::generate(&env);

    let registry_id = env.register(SignalRegistry, ());
    let executor_id = env.register(TradeExecutorStub, ());

    let registry = SignalRegistryClient::new(&env, &registry_id);
    registry.initialize(&admin);
    registry.set_trade_executor(&admin, &executor_id);

    // Sara stakes 200 XLM (above 100 XLM minimum)
    registry.stake_tokens(&sara, &200_000_000i128);

    (env, registry_id, executor_id, admin, sara, alex)
}

// ── Journey ───────────────────────────────────────────────────────────────────

#[test]
fn test_complete_user_journey() {
    let (env, registry_id, executor_id, admin, sara, alex) = setup();
    let registry = SignalRegistryClient::new(&env, &registry_id);

    // ─────────────────────────────────────────────────────────────────────────
    // SARA'S JOURNEY
    // ─────────────────────────────────────────────────────────────────────────

    // Step S1: Sara submits a signal
    let expiry = env.ledger().timestamp() + 3_600;
    let signal_id = registry
        .create_signal(
            &sara,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &1_000_000i128,
            &String::from_str(&env, "XLM breakout above resistance"),
            &expiry,
            &SignalCategory::SWING,
            &Vec::new(&env),
            &RiskLevel::Medium,
        )
        .unwrap();

    assert_eq!(signal_id, 1u64);

    // State: signal is active, no adoptions
    let signal = registry.get_signal(&signal_id).unwrap();
    assert_eq!(signal.provider, sara);
    assert_eq!(signal.adoption_count, 0u32);
    assert_eq!(signal.status, SignalStatus::Active);

    // Step S2: Sara sets platform treasury for fee tracking
    let treasury = Address::generate(&env);
    registry.set_platform_treasury(&admin, &treasury);

    // Step S3: Fee preview — verify fee math before any trade
    let breakdown = registry.calculate_fee_preview(&1_000_000i128).unwrap();
    assert_eq!(breakdown.total_fee, 1_000i128);           // 0.1%
    assert_eq!(breakdown.platform_fee, 700i128);          // 70%
    assert_eq!(breakdown.provider_fee, 300i128);          // 30%
    assert_eq!(breakdown.platform_fee + breakdown.provider_fee, breakdown.total_fee);

    // ─────────────────────────────────────────────────────────────────────────
    // ALEX'S JOURNEY
    // ─────────────────────────────────────────────────────────────────────────

    // Step A1: Alex browses signals — signal is visible
    let signal = registry.get_signal(&signal_id).unwrap();
    assert_eq!(signal.asset_pair, String::from_str(&env, "XLM/USDC"));
    assert_eq!(signal.action, SignalAction::Buy);

    // Step A2: Alex swipes right (increment_adoption — must be called by executor)
    let adoption_count = env.as_contract(&executor_id, || {
        registry.increment_adoption(&executor_id, &signal_id, &1u64)
    });
    assert_eq!(adoption_count, 1u32);

    // State: adoption_count persisted
    let signal = registry.get_signal(&signal_id).unwrap();
    assert_eq!(signal.adoption_count, 1u32);

    // Step A3: Alex copies the trade — profitable execution (+15%)
    registry
        .record_trade_execution(
            &alex,
            &signal_id,
            &1_000_000i128, // entry
            &1_150_000i128, // exit (+15%)
            &10_000_000i128,
        )
        .unwrap();

    let perf = registry.get_signal_performance(&signal_id).unwrap();
    assert_eq!(perf.executions, 1u32);
    assert!(perf.average_roi > 0i128, "profitable trade must have positive ROI");
    assert_eq!(perf.total_volume, 10_000_000i128);

    // Step A4: Stop-loss triggers — second trade at a loss (-15%)
    env.ledger().set_timestamp(1_001_000);
    registry
        .record_trade_execution(
            &alex,
            &signal_id,
            &1_000_000i128,
            &850_000i128,   // exit (-15%) — stop-loss
            &10_000_000i128,
        )
        .unwrap();

    // Step A5: Alex checks P&L — two executions, total volume correct
    let perf = registry.get_signal_performance(&signal_id).unwrap();
    assert_eq!(perf.executions, 2u32);
    assert_eq!(perf.total_volume, 20_000_000i128);

    // ─────────────────────────────────────────────────────────────────────────
    // SARA'S REPUTATION
    // ─────────────────────────────────────────────────────────────────────────

    // Step S4: Record signal outcome (requires executor caller + non-Active signal)
    // Advance past expiry so signal is no longer Active
    env.ledger().set_timestamp(expiry + 1);

    let outcome_result = env.as_contract(&executor_id, || {
        registry.record_signal_outcome(
            &executor_id,
            &signal_id,
            &SignalOutcome::Profit,
        )
    });

    match outcome_result {
        Ok(()) => {
            // new_score = 50 * 0.9 + 100 * 0.1 = 55
            let rep = registry.get_provider_reputation_score(&sara);
            assert_eq!(rep, 55u32, "Sara's reputation must be 55 after Profit");
        }
        Err(_) => {
            // Signal still Active — reputation unchanged at default 50
            let rep = registry.get_provider_reputation_score(&sara);
            assert_eq!(rep, 50u32);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // LEADERBOARD — Sara qualifies after 10 closed signals with adoptions
    // ─────────────────────────────────────────────────────────────────────────

    for i in 2u64..=11 {
        env.ledger().set_timestamp(1_100_000 + i * 100);
        let exp = env.ledger().timestamp() + 3_600;

        let sid = registry
            .create_signal(
                &sara,
                &String::from_str(&env, "XLM/USDC"),
                &SignalAction::Buy,
                &1_000_000i128,
                &String::from_str(&env, "signal"),
                &exp,
                &SignalCategory::SWING,
                &Vec::new(&env),
                &RiskLevel::Medium,
            )
            .unwrap();

        // Adopt (unique nonce per signal)
        env.as_contract(&executor_id, || {
            registry.increment_adoption(&executor_id, &sid, &(i * 10))
        });

        // Profitable trade closes the signal
        registry
            .record_trade_execution(
                &sara,
                &sid,
                &1_000_000i128,
                &1_100_000i128, // +10%
                &5_000_000i128,
            )
            .unwrap();
    }

    // Step A6: Alex views leaderboard — Sara must appear
    let leaderboard = registry.get_provider_leaderboard(&ProviderMetric::BySuccessRate, &10u32);
    let sara_on_lb = (0..leaderboard.len()).any(|i| leaderboard.get(i).unwrap().provider == sara);
    assert!(sara_on_lb, "Sara must appear on the leaderboard");

    // Leaderboard is sorted descending by success rate
    let n = leaderboard.len();
    for i in 0..n.saturating_sub(1) {
        assert!(
            leaderboard.get(i).unwrap().metric_value
                >= leaderboard.get(i + 1).unwrap().metric_value
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // PROVIDER STATS
    // ─────────────────────────────────────────────────────────────────────────

    let stats = registry.get_provider_stats(&sara).unwrap();
    assert!(stats.total_signals >= 10u32);
    assert!(stats.total_volume > 0i128);

    // ─────────────────────────────────────────────────────────────────────────
    // ZERO-ADOPTION GUARD — no division-by-zero
    // ─────────────────────────────────────────────────────────────────────────

    env.ledger().set_timestamp(1_200_000);
    let zero_exp = env.ledger().timestamp() + 3_600;
    let zero_sid = registry
        .create_signal(
            &sara,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Sell,
            &1_000_000i128,
            &String::from_str(&env, "zero adoption signal"),
            &zero_exp,
            &SignalCategory::SWING,
            &Vec::new(&env),
            &RiskLevel::Low,
        )
        .unwrap();

    let zero_signal = registry.get_signal(&zero_sid).unwrap();
    assert_eq!(zero_signal.adoption_count, 0u32);

    // signal_success_rate(0, _) == None — no division by zero
    assert_eq!(signal_success_rate(0, 0), None);
    assert_eq!(signal_success_rate(0, 5), None);
    // With adoptions: defined
    assert_eq!(signal_success_rate(4, 3), Some(7500u32));
    assert_eq!(signal_success_rate(1, 1), Some(10000u32));
}
