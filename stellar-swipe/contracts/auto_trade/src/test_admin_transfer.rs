#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

#[test]
fn test_propose_admin_transfer_auto_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTrade);
    let client = AutoTradeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Initialize contract
    client.init_admin(&admin);

    // Propose transfer - should succeed
    client.propose_admin_transfer(&admin, &new_admin);

    // NewAdmin should NOT be able to set_guardian yet
    let result = client.try_set_guardian(&new_admin, &Address::generate(&env));
    assert!(result.is_err(), "New admin should not have admin privileges before accepting");
}

#[test]
fn test_accept_admin_transfer_auto_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTrade);
    let client = AutoTradeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.init_admin(&admin);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Accept transfer
    client.accept_admin_transfer(&new_admin);

    // Now new_admin should be able to execute admin functions
    let guardian = Address::generate(&env);
    client.set_guardian(&new_admin, &guardian);
    println!("Auto_trade: Transfer completed, new_admin is now admin");
}

#[test]
fn test_accept_with_wrong_address_auto_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTrade);
    let client = AutoTradeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let wrong_address = Address::generate(&env);

    client.init_admin(&admin);
    client.propose_admin_transfer(&admin, &new_admin);

    // Wrong address tries to accept
    let result = client.try_accept_admin_transfer(&wrong_address);
    assert!(result.is_err(), "Wrong address cannot accept transfer");
}

#[test]
fn test_cancel_admin_transfer_auto_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTrade);
    let client = AutoTradeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.init_admin(&admin);
    client.propose_admin_transfer(&admin, &new_admin);

    // Cancel transfer
    client.cancel_admin_transfer(&admin);

    // Accepting should now fail
    let result = client.try_accept_admin_transfer(&new_admin);
    assert!(result.is_err(), "Cannot accept after cancellation");
}

#[test]
fn test_transfer_expiry_auto_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTrade);
    let client = AutoTradeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.init_admin(&admin);

    let initial_timestamp = env.ledger().timestamp();
    client.propose_admin_transfer(&admin, &new_admin);

    // Jump forward 48+ hours
    env.ledger().with_mut(|l| {
        l.timestamp = initial_timestamp + 48 * 60 * 60 + 1;
    });

    // Accepting expired transfer should fail
    let result = client.try_accept_admin_transfer(&new_admin);
    assert!(result.is_err(), "Cannot accept expired transfer");
}
