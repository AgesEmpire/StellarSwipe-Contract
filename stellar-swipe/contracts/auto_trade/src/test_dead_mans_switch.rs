#![cfg(test)]
//! Tests for the admin dead man's switch: inactivity-triggered automatic pause.

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let contract_id = env.register(AutoTradeContract, ());
    let client = AutoTradeContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, contract_id, admin)
}

// ── Timer tracking ────────────────────────────────────────────────────────────

#[test]
fn test_last_admin_action_updated_on_admin_call() {
    let (env, contract_id, admin) = setup();
    let client = AutoTradeContractClient::new(&env, &contract_id);
    let guardian = Address::generate(&env);

    // Advance time then perform an admin action.
    env.ledger().with_mut(|l| l.timestamp = 5_000);
    client.set_guardian(&admin, &guardian);

    let recorded = client.get_last_admin_action_at();
    assert_eq!(recorded, 5_000, "timestamp must reflect the admin action");
}

#[test]
fn test_set_inactivity_window_persisted() {
    let (env, contract_id, admin) = setup();
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let custom_window: u64 = 7 * 24 * 60 * 60; // 7 days
    client.set_inactivity_window(&admin, &custom_window);
    assert_eq!(client.get_inactivity_window(), custom_window);
}

#[test]
fn test_non_admin_cannot_set_inactivity_window() {
    let (env, contract_id, _admin) = setup();
    let client = AutoTradeContractClient::new(&env, &contract_id);
    let attacker = Address::generate(&env);

    let result = client.try_set_inactivity_window(&attacker, &3600u64);
    assert!(
        result.is_err(),
        "non-admin must not configure the inactivity window"
    );
}

// ── Trigger before window elapses (must be rejected) ─────────────────────────

#[test]
fn test_trigger_rejected_before_window_elapses() {
    let (env, contract_id, admin) = setup();
    let client = AutoTradeContractClient::new(&env, &contract_id);
    let anyone = Address::generate(&env);

    // Admin performs an action at t=1000; window = 30 days.
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.set_guardian(&admin, &Address::generate(&env));

    // Advance only 1 day — window (30 days) has not elapsed.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 86_400);

    let result = client.try_trigger_inactivity_pause(&anyone);
    assert!(
        result.is_err(),
        "trigger must be rejected while window has not elapsed"
    );

    // Confirm no pause was applied.
    let states = client.get_pause_states();
    assert!(
        states.get(String::from_str(&env, "all")).is_none(),
        "contract must not be paused"
    );
}

// ── Trigger after window elapses (must succeed) ───────────────────────────────

#[test]
fn test_trigger_succeeds_after_window_elapses() {
    let (env, contract_id, admin) = setup();
    let client = AutoTradeContractClient::new(&env, &contract_id);
    let anyone = Address::generate(&env);

    // Admin acts at t=1000; set short window of 100 s for the test.
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.set_inactivity_window(&admin, &100u64);
    client.set_guardian(&admin, &Address::generate(&env));

    // Advance past the window.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 101);

    client.trigger_inactivity_pause(&anyone);

    // Contract must now be paused under CAT_ALL.
    let states = client.get_pause_states();
    let all_pause = states
        .get(String::from_str(&env, "all"))
        .expect("CAT_ALL pause must be set");
    assert!(all_pause.paused, "contract must be paused");
}

// ── Timer reset by a fresh admin action ───────────────────────────────────────

#[test]
fn test_admin_action_resets_timer_so_trigger_is_rejected_again() {
    let (env, contract_id, admin) = setup();
    let client = AutoTradeContractClient::new(&env, &contract_id);
    let anyone = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.set_inactivity_window(&admin, &100u64);
    // First admin action.
    client.set_guardian(&admin, &Address::generate(&env));

    // Window elapses → trigger would succeed.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 101);

    // Admin re-engages, resetting the timer at t=1101.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 101);
    client.revoke_guardian(&admin);

    // Immediately after the admin action, trigger must be rejected.
    let result = client.try_trigger_inactivity_pause(&anyone);
    assert!(
        result.is_err(),
        "trigger must be rejected immediately after admin re-engages"
    );
}

// ── Unpause via normal admin path after dead man's switch fires ────────────────

#[test]
fn test_admin_can_unpause_after_inactivity_trigger() {
    let (env, contract_id, admin) = setup();
    let client = AutoTradeContractClient::new(&env, &contract_id);
    let anyone = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.set_inactivity_window(&admin, &100u64);
    client.set_guardian(&admin, &Address::generate(&env));
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 101);

    // Trigger the automatic pause.
    client.trigger_inactivity_pause(&anyone);

    // Admin re-engages: unpause via normal path.
    client.unpause_category(&admin, &String::from_str(&env, "all"));

    let states = client.get_pause_states();
    assert!(
        states.get(String::from_str(&env, "all")).is_none(),
        "pause must be lifted after admin unpauses"
    );
}
