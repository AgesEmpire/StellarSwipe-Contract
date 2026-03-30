#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

#[test]
fn test_propose_admin_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer - should succeed
    client.propose_admin_transfer(&admin, &new_admin);

    // Verify admin hasn't changed yet
    // (No direct getter, but trying to set another param with new_admin should fail)
    let result = client.try_set_trade_fee(&new_admin, &25);
    assert!(result.is_err()); // new_admin is not yet the admin
}

#[test]
fn test_accept_admin_transfer_success() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Accept transfer with new_admin
    client.accept_admin_transfer(&new_admin);

    // Now new_admin should be able to execute admin-only functions
    client.set_trade_fee(&new_admin, &25);
    println!("Transfer completed: new_admin is now the admin");
}

#[test]
fn test_accept_admin_transfer_wrong_address() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let wrong_address = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer for new_admin
    client.propose_admin_transfer(&admin, &new_admin);

    // Try to accept with wrong address - should fail
    let result = client.try_accept_admin_transfer(&wrong_address);
    assert!(result.is_err(), "Wrong address should not be able to accept");
}

#[test]
fn test_accept_admin_transfer_no_pending() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let random_address = Address::generate(&env);

    client.initialize(&admin);

    // Try to accept without any pending transfer - should fail
    let result = client.try_accept_admin_transfer(&random_address);
    assert!(result.is_err(), "Should fail when no pending transfer exists");
}

#[test]
fn test_transfer_expiry() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Jump forward 48 hours + 1 second (172800 seconds + 1)
    env.ledger().with_mut(|l| {
        l.timestamp = l.timestamp + 48 * 60 * 60 + 1;
    });

    // Try to accept after expiry - should fail
    let result = client.try_accept_admin_transfer(&new_admin);
    assert!(result.is_err(), "Acceptance should fail after 48 hour expiry");
}

#[test]
fn test_transfer_expiry_boundary() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);

    let initial_timestamp = env.ledger().timestamp();

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Jump forward exactly 48 hours
    env.ledger().with_mut(|l| {
        l.timestamp = initial_timestamp + 48 * 60 * 60;
    });

    // Try to accept exactly at expiry boundary - should fail
    let result = client.try_accept_admin_transfer(&new_admin);
    assert!(result.is_err(), "Acceptance should fail at exact expiry time");

    // Go back and try just before expiry
    let contract_id2 = env.register_contract(None, SignalRegistry);
    let client2 = SignalRegistryClient::new(&env, &contract_id2);

    client2.initialize(&admin);
    client2.propose_admin_transfer(&admin, &new_admin);

    env.ledger().with_mut(|l| {
        l.timestamp = initial_timestamp + 48 * 60 * 60 - 1; // 1 second before expiry
    });

    // Should succeed just before expiry
    client2.accept_admin_transfer(&new_admin);
    println!("Transfer succeeded just before expiry");
}

#[test]
fn test_cancel_admin_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Cancel it
    client.cancel_admin_transfer(&admin);

    // Try to accept - should fail (no pending transfer anymore)
    let result = client.try_accept_admin_transfer(&new_admin);
    assert!(result.is_err(), "Should not be able to accept after cancellation");
}

#[test]
fn test_cancel_admin_transfer_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let random_address = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Non-admin tries to cancel - should fail
    let result = client.try_cancel_admin_transfer(&random_address);
    assert!(result.is_err(), "Only admin should be able to cancel");
}

#[test]
fn test_cancel_admin_transfer_no_pending() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Try to cancel when no transfer pending - should fail
    let result = client.try_cancel_admin_transfer(&admin);
    assert!(result.is_err(), "Should fail when no pending transfer exists");
}

#[test]
fn test_multiple_transfer_proposals() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin_1 = Address::generate(&env);
    let new_admin_2 = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer to new_admin_1
    client.propose_admin_transfer(&admin, &new_admin_1);

    // Propose transfer to new_admin_2 (replaces previous)
    client.propose_admin_transfer(&admin, &new_admin_2);

    // new_admin_1 tries to accept - should fail (no longer pending)
    let result = client.try_accept_admin_transfer(&new_admin_1);
    assert!(result.is_err(), "First address should not be able to accept after new proposal");

    // new_admin_2 should be able to accept
    client.accept_admin_transfer(&new_admin_2);
    println!("Second admin accepted transfer successfully");
}

#[test]
fn test_transfer_chain() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);

    client.initialize(&admin1);

    // Transfer from admin1 to admin2
    client.propose_admin_transfer(&admin1, &admin2);
    client.accept_admin_transfer(&admin2);

    // Transfer from admin2 to admin3
    client.propose_admin_transfer(&admin2, &admin3);
    client.accept_admin_transfer(&admin3);

    // Verify admin3 is now the admin
    client.set_trade_fee(&admin3, &30);
    println!("Admin transfer chain completed successfully");
}

#[test]
fn test_old_admin_cannot_transfer_after_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    client.initialize(&admin1);

    // Transfer from admin1 to admin2
    client.propose_admin_transfer(&admin1, &admin2);
    client.accept_admin_transfer(&admin2);

    // admin1 tries to execute admin function - should fail
    let result = client.try_set_trade_fee(&admin1, &35);
    assert!(result.is_err(), "Old admin should not be able to act after transfer");
}

#[test]
fn test_propose_no_pending_cleanup_on_expired() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin1 = Address::generate(&env);
    let new_admin2 = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer to new_admin1
    let initial_timestamp = env.ledger().timestamp();
    client.propose_admin_transfer(&admin, &new_admin1);

    // Jump forward 48+ hours
    env.ledger().with_mut(|l| {
        l.timestamp = initial_timestamp + 48 * 60 * 60 + 1;
    });

    // Try to accept expired - should fail and clean up
    let result = client.try_accept_admin_transfer(&new_admin1);
    assert!(result.is_err());

    // Propose new transfer should work
    client.propose_admin_transfer(&admin, &new_admin2);

    // new_admin2 should be able to accept (no interference from expired proposal)
    client.accept_admin_transfer(&new_admin2);
    println!("New transfer succeeded after cleanup of expired transfer");
}
