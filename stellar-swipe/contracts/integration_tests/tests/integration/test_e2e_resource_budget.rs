//! End-to-end CPU instruction budget checks for multi-contract workflows
//! (Issue #1342).
//!
//! Existing budget-regression coverage (`common::budget_regression`,
//! `signal_registry::query`, `trade_executor::tests::test_latency_benchmark`,
//! etc.) measures isolated entrypoints in a single contract. A realistic
//! workflow composes several calls across a contract (and, via
//! `increment_adoption`/`record_trade_execution`, across the caller
//! boundary used in production by `trade_executor`), so its *total*
//! instruction cost can exceed ledger limits even when every individual
//! call stays within budget. These tests measure representative end-to-end
//! workflows instead of isolated calls, using realistic maximum bounded
//! inputs, and fail CI with the actual vs. baseline instruction count when a
//! workflow regresses.
//!
//! # Updating a baseline
//! If a regression is intentional, re-run this test, read the printed
//! `BUDGET_METRIC` line for the new instruction count, and update the
//! corresponding baseline constant below alongside the change that caused
//! the increase (document why in the commit message).

extern crate std;

use signal_registry::{
    RiskLevel, SignalAction, SignalCategory, SignalRegistry, SignalRegistryClient,
};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{budget::Budget, Address as _, Ledger},
    Address, Env, String, Vec,
};
use stellar_swipe_common::budget_regression::measure_and_emit;

#[contract]
struct TradeExecutorStub;

#[contractimpl]
impl TradeExecutorStub {}

/// End-to-end signal-provider workflow: create a signal, have it adopted,
/// and record a bounded number of trade executions against it — the same
/// sequence `test_complete_user_journey` exercises functionally, but here
/// measured for total instruction cost rather than correctness.
///
/// Baseline derived from a clean run on `soroban-sdk = "23"`; see module
/// doc for the update procedure.
const SIGNAL_WORKFLOW_BASELINE_INSTRUCTIONS: u64 = 15_000_000;

/// Realistic maximum bound on executions recorded against a single signal
/// within one workflow measurement, chosen to exercise a non-trivial loop
/// rather than a single call.
const MAX_EXECUTIONS_PER_WORKFLOW: u32 = 20;

#[test]
fn signal_registry_create_adopt_execute_workflow_stays_within_budget() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let follower = Address::generate(&env);

    let registry_id = env.register(SignalRegistry, ());
    let executor_id = env.register(TradeExecutorStub, ());
    let registry = SignalRegistryClient::new(&env, &registry_id);

    registry.initialize(&admin);
    registry.stake_tokens(&provider, &200_000_000i128);

    let expiry = env.ledger().timestamp() + 3_600;
    let signal_id = registry.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, "end-to-end budget workflow"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );

    // Measure from here: adoption + a realistic maximum run of recorded
    // trade executions is the "workflow," not just signal creation.
    env.budget().reset_tracker();

    env.as_contract(&executor_id, || {
        registry.increment_adoption(&executor_id, &signal_id, &1u64)
    });

    for i in 0..MAX_EXECUTIONS_PER_WORKFLOW {
        registry.record_trade_execution(
            &follower,
            &signal_id,
            &1_000_000i128,
            &1_050_000i128,
            &10_000_000i128,
        );
        let _ = i;
    }

    let instructions = env.budget().cpu_instruction_cost();
    measure_and_emit(
        "signal_registry_create_adopt_execute_workflow",
        SIGNAL_WORKFLOW_BASELINE_INSTRUCTIONS,
        instructions,
    );

    // Sanity: the workflow actually did the work being measured (a budget
    // baseline for a no-op workflow would be meaningless).
    let perf = registry
        .get_signal_performance(&signal_id)
        .expect("performance must be recorded after executions");
    assert_eq!(perf.executions, MAX_EXECUTIONS_PER_WORKFLOW);
}

/// Storage-growth workflow: repeatedly create signals from the same
/// provider up to a realistic per-call-sequence bound, which is the
/// multi-contract-adjacent pattern that most directly stresses persistent
/// storage write/read budgets end-to-end (every `create_signal` reads and
/// rewrites the provider's signal map).
const SIGNAL_CREATION_BURST_BASELINE_INSTRUCTIONS: u64 = 20_000_000;
const SIGNAL_CREATION_BURST_COUNT: u32 = 25;

#[test]
fn signal_registry_creation_burst_storage_workflow_stays_within_budget() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    let registry_id = env.register(SignalRegistry, ());
    let registry = SignalRegistryClient::new(&env, &registry_id);
    registry.initialize(&admin);
    registry.stake_tokens(&provider, &200_000_000i128);

    env.budget().reset_tracker();

    let mut last_id = 0u64;
    for i in 0..SIGNAL_CREATION_BURST_COUNT {
        env.ledger().set_timestamp(1_000_000 + (i as u64) * 10);
        let expiry = env.ledger().timestamp() + 3_600;
        last_id = registry.create_signal(
            &provider,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &1_000_000i128,
            &String::from_str(&env, "storage burst workflow"),
            &expiry,
            &SignalCategory::SWING,
            &Vec::new(&env),
            &RiskLevel::Medium,
        );
    }

    let instructions = env.budget().cpu_instruction_cost();
    measure_and_emit(
        "signal_registry_creation_burst_storage_workflow",
        SIGNAL_CREATION_BURST_BASELINE_INSTRUCTIONS,
        instructions,
    );

    assert_eq!(last_id, SIGNAL_CREATION_BURST_COUNT as u64);
}
