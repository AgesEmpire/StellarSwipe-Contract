#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

fn xlm_asset(env: &Env) -> stellar_swipe_common::Asset {
    stellar_swipe_common::Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    }
}

#[test]
fn test_propose_admin_transfer_oracle() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Initialize the contract first
    let base_asset = xlm_asset(&env);
    client.initialize(&admin, &base_asset);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);
}

#[test]
fn test_accept_admin_transfer_oracle() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Initialize the contract first
    let base_asset = xlm_asset(&env);
    client.initialize(&admin, &base_asset);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Accept transfer
    client.accept_admin_transfer(&new_admin);

    // Unpause should work (new_admin is now admin)
    client.unpause_category(&new_admin, &soroban_sdk::String::from_str(&env, "test_category"));
}

#[test]
fn test_cancel_admin_transfer_oracle() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Initialize the contract first
    let base_asset = xlm_asset(&env);
    client.initialize(&admin, &base_asset);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Cancel transfer
    client.cancel_admin_transfer(&admin);

    // Accept should fail
    let result = client.try_accept_admin_transfer(&new_admin);
    assert!(result.is_err(), "Cannot accept after cancellation");
}

#[test]
fn test_transfer_expiry_oracle() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Initialize the contract first
    let base_asset = xlm_asset(&env);
    client.initialize(&admin, &base_asset);

    let initial_timestamp = env.ledger().timestamp();
    client.propose_admin_transfer(&admin, &new_admin);

    // Jump forward 48+ hours
    env.ledger().with_mut(|l| {
        l.timestamp = initial_timestamp + 48 * 60 * 60 + 1;
    });

    // Expired transfer cannot be accepted
    let result = client.try_accept_admin_transfer(&new_admin);
    assert!(result.is_err(), "Cannot accept expired transfer");
}

#[test]
fn test_accept_with_wrong_address_oracle() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let wrong_address = Address::generate(&env);

    // Initialize the contract first
    let base_asset = xlm_asset(&env);
    client.initialize(&admin, &base_asset);

    client.propose_admin_transfer(&admin, &new_admin);

    // Wrong address tries to accept
    let result = client.try_accept_admin_transfer(&wrong_address);
    assert!(result.is_err(), "Wrong address cannot accept transfer");
}
