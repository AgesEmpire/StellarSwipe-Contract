#![cfg(test)]

use super::AutoTradeContract;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};
use stellar_swipe_common::emergency::CAT_ALL;

#[test]
fn health_not_initialized() {
    let env = Env::default();
    let id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &id);
    let h = client.health_check();
    assert!(!h.is_initialized);
    assert!(!h.is_paused);
}

#[test]
fn health_initialized_running() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &id);
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
    let id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    client
        .pause_category(
            &admin,
            &String::from_str(&env, CAT_ALL),
            &None,
            &String::from_str(&env, "test"),
        )
        .expect("pause");

    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(h.is_paused);
    assert_eq!(h.admin, admin);
}
