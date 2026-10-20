#![cfg(test)]

use crate::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Vec,
};

#[test]
fn test_increment_adoption() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

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

    let mut signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.adoption_count, 0);

    let nonce1 = 1u64;
    let count1 = client.increment_adoption(&executor, &signal_id, &nonce1);
    assert_eq!(count1, 1);

    signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.adoption_count, 1);

    let dup = client.try_increment_adoption(&executor, &signal_id, &nonce1);
    assert!(dup.is_err());

    let nonce2 = 2u64;
    let count2 = client.increment_adoption(&executor, &signal_id, &nonce2);
    assert_eq!(count2, 2);

    signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.adoption_count, 2);
}

#[test]
fn test_wrong_caller() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);
    let auth_executor = Address::generate(&env);
    client.set_trade_executor(&admin, &auth_executor);

    let provider = Address::generate(&env);
    let wrong_caller = Address::generate(&env);

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

    let nonce = 1u64;
    let r = client.try_increment_adoption(&wrong_caller, &signal_id, &nonce);
    assert!(r.is_err());
}

#[test]
fn test_adoption_on_inactive_signal() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

    let tags = Vec::new(&env);
    let expiry = env.ledger().timestamp() + 86400;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    env.ledger().set_timestamp(expiry + 1);
    client.cleanup_expired_signals(&100);

    let nonce = 1u64;
    let r = client.try_increment_adoption(&executor, &signal_id, &nonce);
    assert!(r.is_err());
}

#[test]
fn signal_adopted_event_has_notification_fields() {
    use soroban_sdk::testutils::{Events, Ledger};
    use soroban_sdk::TryFromVal;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 7777);

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

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

    client.increment_adoption(&executor, &signal_id, &1u64);

    let events = env.events().all();
    let adopted_evt = events.iter().find(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        if topics.len() < 2 {
            return false;
        }
        let t1 = soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap());
        t1.map(|s| s == soroban_sdk::Symbol::new(&env, "signal_adopted"))
            .unwrap_or(false)
    });

    assert!(adopted_evt.is_some(), "EvtSignalAdopted not emitted");
    let evt: shared::events::EvtSignalAdopted =
        shared::events::EvtSignalAdopted::try_from_val(&env, &adopted_evt.unwrap().2).unwrap();

    assert_eq!(evt.signal_id, signal_id);
    assert_eq!(evt.new_count, 1);
    assert_eq!(evt.timestamp, 7777);
    assert!(!evt.action_required);
    assert_eq!(evt.schema_version, shared::events::SCHEMA_VERSION);
}
