#![cfg(test)]
extern crate std;

// Property-based tests for signal validation boundary conditions (issue #780).
//
// `calculate_quality_score` / `get_signal_quality_score` combine four
// independently-bounded components (success rate, adoption, stake tier, AI
// score) via basis-point weighted arithmetic. Unit tests only pin a handful
// of fixed inputs; this asserts the output invariant — score in `[0, 100]`
// — holds across the whole input space, including the extremes that are
// easy to miss by hand (zero executions, max adoption, no stake, etc).

use crate::categories::{RiskLevel, SignalCategory};
use crate::scoring::{calculate_quality_score, get_signal_quality_score};
use crate::stake::StakeInfo;
use crate::types::{Signal, SignalAction, SignalStatus};
use crate::{SignalRegistry, StorageKey};
use proptest::prelude::*;
use soroban_sdk::{testutils::Address as _, Address, Env, Map, String, Vec};

fn sdk_string(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
    let env = Env::default();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    env.as_contract(&contract_id, || f(&env))
}

fn setup_stake(env: &Env, provider: &Address, amount: i128) {
    let mut stakes: Map<Address, StakeInfo> = Map::new(env);
    stakes.set(
        provider.clone(),
        StakeInfo {
            amount,
            last_signal_time: 0,
            locked_until: 0,
        },
    );
    env.storage()
        .instance()
        .set(&StorageKey::ProviderStakes, &stakes);
}

#[allow(clippy::too_many_arguments)]
fn make_signal(
    env: &Env,
    id: u64,
    provider: Address,
    executions: u32,
    successful_executions: u32,
    adoption_count: u32,
    ai_score: Option<u32>,
) -> Signal {
    Signal {
        id,
        provider,
        asset_pair: sdk_string(env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100_000_000,
        rationale: sdk_string(env, "Property test signal"),
        timestamp: 0,
        expiry: 86_400,
        status: SignalStatus::Active,
        executions,
        successful_executions,
        total_volume: 0,
        total_roi: 0,
        category: SignalCategory::SWING,
        tags: Vec::new(env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
        submitted_at: 0,
        rationale_hash: sdk_string(env, "hash"),
        confidence: 50,
        adoption_count,
        ai_validation_score: ai_score,
        avg_copier_roi_bps: 0,
        copier_closed_count: 0,
        warning_emitted: false,
        benchmark_return_bps: None,
        alpha_bps: None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// `calculate_quality_score` must never exceed 100, for any combination
    /// of execution counts, adoption count, stake amount, or (optional) AI
    /// score — including the `executions == 0` and `ai_score == None`
    /// branches, which use different weight redistribution logic.
    #[test]
    fn quality_score_always_at_most_100(
        executions in 0u32..=1_000u32,
        successful_delta in 0u32..=1_000u32,
        adoption_count in 0u32..=1_000_000u32,
        stake_amount in 0i128..=10_000_000_000i128,
        ai_score in proptest::option::of(0u32..=1_000u32),
    ) {
        // successful_executions must never exceed executions (real invariant
        // maintained by the contract's execution-recording path).
        let successful_executions = successful_delta.min(executions);

        with_contract(|env| -> Result<(), TestCaseError> {
            let provider = Address::generate(env);
            setup_stake(env, &provider, stake_amount);

            let signal = make_signal(
                env,
                1,
                provider,
                executions,
                successful_executions,
                adoption_count,
                ai_score,
            );

            let score = calculate_quality_score(env, &signal);
            prop_assert!(score <= 100, "calculate_quality_score returned {}", score);

            // Also exercise the public, storage-backed entry point named in
            // issue #780: store the signal and read the score back through
            // `get_signal_quality_score`.
            let mut signals: Map<u64, Signal> = Map::new(env);
            signals.set(signal.id, signal.clone());
            env.storage().instance().set(&StorageKey::Signals, &signals);

            let stored_score = get_signal_quality_score(env, signal.id);
            prop_assert_eq!(stored_score, Some(score));
            prop_assert!(stored_score.unwrap() <= 100);

            Ok(())
        })?;
    }
}
