//! Tests for Issue #1421: freeze contest entrants when a contest starts.
#![cfg(test)]
use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::contests::ContestStatus;
use soroban_sdk::{testutils::{Address as _, Ledger}, vec, Address, Env, String};

fn advance_past_randomness(env: &Env) {
    env.ledger().with_mut(|li| li.sequence_number += 6);
}

fn setup(env: &Env) -> (Address, SignalRegistryClient) {
    env.mock_all_auths();
    env.ledger().set_timestamp(10_000);
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (admin, client)
}

fn make_signal(env: &Env, client: &SignalRegistryClient, provider: &Address) -> u64 {
    client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &100,
        &String::from_str(env, "test rationale"),
        &(env.ledger().timestamp() + 3600),
        &SignalCategory::SWING,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Medium,
    )
}

/// `start_contest` must capture all current entrants in a sorted snapshot.
#[test]
fn test_start_contest_captures_entrant_snapshot() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    let p3 = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;
    let cid = client.create_contest(
        &admin,
        &String::from_str(&env, "Freeze Test"),
        &start_time,
        &end_time,
        &crate::contests::ContestMetric::HighestROI,
        &1,
        &1000,
    );

    // Three providers submit signals → auto-entered.
    make_signal(&env, &client, &p1);
    make_signal(&env, &client, &p2);
    make_signal(&env, &client, &p3);

    let snapshot = client.start_contest(&cid);

    // All three entrants must appear in the snapshot.
    assert_eq!(snapshot.len(), 3);

    // Snapshot must be sorted (lexicographic on address string).
    for i in 0..snapshot.len().saturating_sub(1) {
        let a = snapshot.get(i).unwrap().to_string().to_bytes();
        let b = snapshot.get(i + 1).unwrap().to_string().to_bytes();
        assert!(a <= b, "snapshot is not sorted at index {i}");
    }

    // Contest status must transition to Started.
    let contest = client.get_contest(&cid);
    assert_eq!(contest.status, ContestStatus::Started);
}

/// After `start_contest`, new entry attempts must not change contest state.
#[test]
fn test_post_start_entry_attempt_is_rejected() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let p1 = Address::generate(&env);
    let late_provider = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;
    let cid = client.create_contest(
        &admin,
        &String::from_str(&env, "Post-start Test"),
        &start_time,
        &end_time,
        &crate::contests::ContestMetric::HighestROI,
        &1,
        &1000,
    );

    make_signal(&env, &client, &p1);
    client.start_contest(&cid);

    // A signal from a provider who wasn't enrolled before start should NOT appear
    // in the entries map after auto_enter_signal runs.
    make_signal(&env, &client, &late_provider);

    let contest = client.get_contest(&cid);
    assert!(
        contest.entries.get(late_provider).is_none(),
        "late provider must not be enrolled after contest start"
    );
}

/// Calling `start_contest` twice must fail without changing state.
#[test]
fn test_start_contest_twice_fails() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;
    let cid = client.create_contest(
        &admin,
        &String::from_str(&env, "Double Start"),
        &start_time,
        &end_time,
        &crate::contests::ContestMetric::HighestROI,
        &1,
        &1000,
    );

    client.start_contest(&cid);
    let result = client.try_start_contest(&cid);
    assert!(result.is_err(), "second start_contest must fail");
}

/// `start_contest` on an empty contest produces an empty snapshot but still transitions status.
#[test]
fn test_start_contest_empty_entrant_set() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;
    let cid = client.create_contest(
        &admin,
        &String::from_str(&env, "Empty Contest"),
        &start_time,
        &end_time,
        &crate::contests::ContestMetric::HighestROI,
        &1,
        &1000,
    );

    let snapshot = client.start_contest(&cid);
    assert_eq!(snapshot.len(), 0);
    assert_eq!(client.get_contest(&cid).status, ContestStatus::Started);
}

/// Finalization after `start_contest` uses only the snapshotted entrants.
#[test]
fn test_finalize_uses_snapshot() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let p_early = Address::generate(&env);
    let p_late = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 100;
    let cid = client.create_contest(
        &admin,
        &String::from_str(&env, "Snapshot Finalize"),
        &start_time,
        &end_time,
        &crate::contests::ContestMetric::HighestROI,
        &1,
        &1000,
    );

    let sid = make_signal(&env, &client, &p_early);
    client.record_trade_execution(&p_early, &sid, &100, &110, &1000);
    client.start_contest(&cid);

    // Late provider submits after start — should not appear in winners.
    make_signal(&env, &client, &p_late);

    env.ledger().set_timestamp(end_time + 1);
    advance_past_randomness(&env);

    let winners = client.finalize_contest(&cid);
    assert_eq!(winners.len(), 1);
    assert_eq!(winners.get(0).unwrap(), p_early);
}
