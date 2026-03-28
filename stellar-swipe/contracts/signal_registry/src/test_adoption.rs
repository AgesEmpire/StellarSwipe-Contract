#![cfg(test)]

use crate::test_utils::setup_env;
use crate::*;
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol, Vec};

#[test]
fn test_increment_adoption() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let executor = Address::generate(&env); // TradeExecutor

    // Create signal
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Test"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    // Initial adoption count 0
    let mut signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.adoption_count, 0);

    // Increment 1
    let nonce1 = 1u64;
    let count1 = client.increment_adoption(&executor, &signal_id, &nonce1);
    assert_eq!(count1, 1);

    // Check count updated
    signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.adoption_count, 1);

    // Duplicate nonce fails
    let result = client.increment_adoption(&executor, &signal_id, &nonce1);
    assert!(result.is_err());

    // New nonce succeeds
    let nonce2 = 2u64;
    let count2 = client.increment_adoption(&executor, &signal_id, &nonce2);
    assert_eq!(count2, 2);

    signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.adoption_count, 2);

    // Verify event emitted
    let events = env.events().all();
    assert_eq!(events.len(), 1); // adoption event
    let event = events.get(0).unwrap();
    assert_eq!(event.0.get(1).unwrap().u64().unwrap(), signal_id); // topic signal_id
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_wrong_caller() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let wrong_caller = Address::generate(&env);

    // Create signal
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Test"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    // Wrong caller
    let nonce = 1u64;
    client.increment_adoption(&wrong_caller, &signal_id, &nonce);
}

#[test]
fn test_adoption_on_inactive_signal() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    // Create signal
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Test"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    // Expire the signal
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + 100_000);
    let mut signals = client.get_signals_map(&env); // Assume helper or direct
                                                    // ... mark as expired

    // Increment should fail
    let nonce = 1u64;
    let result = client.increment_adoption(&executor, &signal_id, &nonce);
    assert!(result.is_err());
}
