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

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Initialize contract
    client.initialize(&admin);

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

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);

    // Propose transfer
    client.propose_admin_transfer(&admin, &new_admin);

    // Accept transfer
    client.accept_admin_transfer(&new_admin);

    // Now new_admin should be able to execute admin functions
    let guardian = Address::generate(&env);
    client.set_guardian(&new_admin, &guardian);
}

#[test]
fn test_accept_with_wrong_address_auto_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let wrong_address = Address::generate(&env);

    client.initialize(&admin);
    client.propose_admin_transfer(&admin, &new_admin);

    // Wrong address tries to accept
    let result = client.try_accept_admin_transfer(&wrong_address);
    assert!(result.is_err(), "Wrong address cannot accept transfer");
}

#[test]
fn test_cancel_admin_transfer_auto_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);
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

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);

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

// ── Issue #267: Admin Privilege Escalation Path Tests ────────────────────────

/// Non-admin cannot propose an admin transfer.
#[test]
fn test_non_admin_cannot_propose_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let victim = Address::generate(&env);

    client.initialize(&admin);

    let result = client.try_propose_admin_transfer(&attacker, &victim);
    assert!(result.is_err(), "Non-admin must not be able to propose admin transfer");
}

/// Guardian cannot propose or accept an admin transfer.
#[test]
fn test_guardian_cannot_escalate_to_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);

    client.initialize(&admin);
    client.set_guardian(&admin, &guardian);

    // Guardian cannot propose a transfer
    let propose_result = client.try_propose_admin_transfer(&guardian, &guardian);
    assert!(propose_result.is_err(), "Guardian must not propose admin transfer");

    // Guardian cannot accept a transfer that was never proposed
    let accept_result = client.try_accept_admin_transfer(&guardian);
    assert!(accept_result.is_err(), "Guardian must not accept non-existent transfer");
}

/// Double-initialization is blocked — admin cannot be overwritten via re-init.
#[test]
fn test_reinitialize_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);

    client.initialize(&admin);

    // Second initialize must panic (already initialized guard)
    let result = std::panic::catch_unwind(|| {
        let env2 = env.clone();
        let client2 = AutoTradeContractClient::new(&env2, &contract_id);
        client2.initialize(&attacker);
    });
    assert!(result.is_err(), "Re-initialization must be blocked");
}

/// Pending admin cannot use admin privileges before accepting.
#[test]
fn test_pending_admin_has_no_privileges_before_accept() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);
    client.propose_admin_transfer(&admin, &new_admin);

    // Pending admin cannot set guardian before accepting
    let result = client.try_set_guardian(&new_admin, &Address::generate(&env));
    assert!(result.is_err(), "Pending admin must not have admin privileges before accepting");

    // Pending admin cannot cancel the transfer
    let cancel_result = client.try_cancel_admin_transfer(&new_admin);
    assert!(cancel_result.is_err(), "Pending admin must not cancel transfer");
}

/// After a completed transfer, the old admin loses all privileges.
#[test]
fn test_old_admin_loses_privileges_after_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, AutoTradeContract);
    let client = AutoTradeContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.initialize(&admin);
    client.propose_admin_transfer(&admin, &new_admin);
    client.accept_admin_transfer(&new_admin);

    // Old admin can no longer set guardian
    let result = client.try_set_guardian(&admin, &Address::generate(&env));
    assert!(result.is_err(), "Old admin must lose privileges after transfer");
}
