#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn health_uninitialized_contract() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let h = client.health_check();
    assert!(!h.is_initialized);
    assert!(!h.is_paused);
}

#[test]
fn health_initialized_running() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(!h.is_paused);
    assert_eq!(h.admin, admin);
}

#[test]
fn health_initialized_paused() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);
    client.pause_trading(&admin);

    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(h.is_paused);
    assert_eq!(h.admin, admin);
}
