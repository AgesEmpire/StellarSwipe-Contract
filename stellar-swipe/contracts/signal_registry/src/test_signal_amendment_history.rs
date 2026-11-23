//! Integration-level tests for Issue #1419: preserve immutable versions of amended signals.
#![cfg(test)]
use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use soroban_sdk::{testutils::{Address as _, Ledger}, vec, Address, Env, String};

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

fn create_signal(env: &Env, client: &SignalRegistryClient, provider: &Address) -> u64 {
    client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &100,
        &String::from_str(env, "original rationale"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Low,
    )
}

// ── monotonic revision identifiers ───────────────────────────────────────────

#[test]
fn test_amendment_produces_monotonic_revision() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let sid = create_signal(&env, &client, &provider);

    let v1 = client.update_signal_versioned(
        &sid,
        &provider,
        &Some(200i128),
        &None,
        &None,
    );

    // Allow cooldown.
    env.ledger().with_mut(|li| li.timestamp += 3601);

    let v2 = client.update_signal_versioned(
        &sid,
        &provider,
        &Some(300i128),
        &None,
        &None,
    );

    assert!(v2 > v1, "each amendment must produce a strictly higher revision");
}

// ── history unchanged after later edits ──────────────────────────────────────

#[test]
fn test_prior_snapshot_unchanged_after_later_amendment() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let sid = create_signal(&env, &client, &provider);

    // First amendment: price 100 → 200.
    client.update_signal_versioned(&sid, &provider, &Some(200i128), &None, &None);

    // Read snapshot stored at revision 1 (the original price=100 state).
    let snap_v1 = client.get_signal_revision(&sid, &1u32).unwrap();
    let recorded_price = snap_v1.price;

    // Second amendment: price 200 → 300.
    env.ledger().with_mut(|li| li.timestamp += 3601);
    client.update_signal_versioned(&sid, &provider, &Some(300i128), &None, &None);

    // The snapshot at revision 1 must not have changed.
    let snap_v1_after = client.get_signal_revision(&sid, &1u32).unwrap();
    assert_eq!(
        snap_v1_after.price,
        recorded_price,
        "revision 1 snapshot must be immutable after a later amendment"
    );
}

// ── current revision query ────────────────────────────────────────────────────

#[test]
fn test_get_current_revision_returns_latest_snapshot() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let sid = create_signal(&env, &client, &provider);

    // No amendments yet.
    assert!(client.get_signal_current_revision(&sid).is_none());

    client.update_signal_versioned(&sid, &provider, &Some(500i128), &None, &None);
    env.ledger().with_mut(|li| li.timestamp += 3601);
    client.update_signal_versioned(&sid, &provider, &Some(600i128), &None, &None);

    // After two amendments the most recent snapshot captured price=500.
    let current = client.get_signal_current_revision(&sid).unwrap();
    assert_eq!(current.price, 500i128);
}

// ── bounded prior revision query ─────────────────────────────────────────────

#[test]
fn test_prior_revision_bounded_lookup() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let sid = create_signal(&env, &client, &provider);

    // Create 3 amendments.
    for price in [200i128, 300, 400] {
        env.ledger().with_mut(|li| li.timestamp += 3601);
        client.update_signal_versioned(&sid, &provider, &Some(price), &None, &None);
    }
    // Latest version = 4; snapshots at versions 1, 2, 3.

    // With max_lookback=2 from latest(4), floor=2; revision 1 clamped to 2.
    let bounded = client.get_signal_prior_revision(&sid, &1u32, &2u32).unwrap();
    assert_eq!(bounded.version, 2);

    // With max_lookback=10, revision 2 passes through.
    let r2 = client.get_signal_prior_revision(&sid, &2u32, &10u32).unwrap();
    assert_eq!(r2.version, 2);
}

// ── start boundary: amendment exactly at start returns revision ───────────────

#[test]
fn test_first_amendment_stores_revision_1() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let sid = create_signal(&env, &client, &provider);

    client.update_signal_versioned(&sid, &provider, &Some(200i128), &None, &None);

    let snap = client.get_signal_revision(&sid, &1u32);
    assert!(snap.is_some(), "revision 1 must exist after first amendment");
    assert_eq!(snap.unwrap().price, 100i128, "revision 1 must capture original price");
}
