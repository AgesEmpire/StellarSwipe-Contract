#![cfg(test)]
//! Representative read-only query coverage for `signal_registry` (Issue #1336).
//!
//! Uses the shared `stellar_swipe_common::read_only_check` helper (added by
//! this same change) instead of each test hand-rolling its own
//! before/after event-count comparison. Covers three representative query
//! shapes: a struct getter (`get_signal`), a view-struct getter
//! (`get_signal_performance`), and a boolean predicate
//! (`is_provider_banned`).

use crate::{SignalAction, SignalCategory, SignalRegistry, SignalRegistryClient, RiskLevel};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};
use stellar_swipe_common::read_only_check::{assert_no_events_published, assert_read_only_stable};

fn setup() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, admin, client)
}

#[test]
fn get_signal_is_read_only() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    client.stake_tokens(&provider, &200_000_000i128);
    let expiry = env.ledger().timestamp() + 3_600;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, "read-only query coverage"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );

    // get_signal returns a non-PartialEq struct, so only the no-events
    // half of the check applies here; field-level stability is covered by
    // get_signal_performance below.
    let signal = assert_no_events_published(&env, "get_signal", || {
        client.get_signal(&signal_id).expect("signal must exist")
    });
    assert_eq!(signal.provider, provider);
}

#[test]
fn get_signal_performance_is_read_only_and_stable() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    client.stake_tokens(&provider, &200_000_000i128);
    let expiry = env.ledger().timestamp() + 3_600;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, "read-only query coverage"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );

    let perf = assert_read_only_stable(&env, "get_signal_performance", || {
        client
            .get_signal_performance(&signal_id)
            .expect("performance view must exist")
    });
    assert_eq!(perf.executions, 0);
}

#[test]
fn is_provider_banned_is_read_only_and_stable() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    client.stake_tokens(&provider, &200_000_000i128);

    let banned = assert_read_only_stable(&env, "is_provider_banned", || {
        client.is_provider_banned(&provider)
    });
    assert!(!banned, "freshly staked provider must not be banned");
}
