#![cfg(test)]

use crate::{OracleContract, OracleContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String};
use stellar_swipe_common::emergency::CAT_ALL;
use stellar_swipe_common::Asset;

fn xlm(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    }
}

#[test]
fn health_not_initialized_without_admin() {
    let env = Env::default();
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let h = client.health_check();
    assert!(!h.is_initialized);
    assert!(!h.is_paused);
}

#[test]
fn health_initialized_and_running() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &xlm(&env));

    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(!h.is_paused);
    assert_eq!(h.admin, admin);
}

#[test]
fn health_initialized_when_paused() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &xlm(&env));

    client.pause_category(
        &admin,
        &String::from_str(&env, CAT_ALL),
        &None,
        &String::from_str(&env, "test"),
    );

    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(h.is_paused);
}
