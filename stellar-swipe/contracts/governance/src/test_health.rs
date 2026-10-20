#![cfg(test)]

use crate::{GovernanceContract, GovernanceContractClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String};

use crate::distribution::DistributionRecipients;

const SUPPLY: i128 = 1_000_000_000;

fn recipients(env: &Env) -> DistributionRecipients {
    DistributionRecipients {
        team: Address::generate(env),
        early_investors: Address::generate(env),
        community_rewards: Address::generate(env),
        treasury: Address::generate(env),
        public_sale: Address::generate(env),
    }
}

#[test]
fn health_not_initialized() {
    let env = Env::default();
    let id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &id);
    let h = client.health_check();
    assert!(!h.is_initialized);
    assert!(!h.is_paused);
}

#[test]
fn health_initialized_running() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let r = recipients(&env);
    let client = GovernanceContractClient::new(&env, &id);
    client.initialize(
        &admin,
        &String::from_str(&env, "StellarSwipe Gov"),
        &String::from_str(&env, "SSG"),
        &7u32,
        &SUPPLY,
        &r,
    );

    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(!h.is_paused);
    assert_eq!(h.admin, admin);
}

#[test]
fn health_initialized_paused() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let r = recipients(&env);
    let client = GovernanceContractClient::new(&env, &id);
    client.initialize(
        &admin,
        &String::from_str(&env, "StellarSwipe Gov"),
        &String::from_str(&env, "SSG"),
        &7u32,
        &SUPPLY,
        &r,
    );

    client.set_contract_paused(&admin, &true);

    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(h.is_paused);
    assert_eq!(h.admin, admin);
}

// ── Issue #884: Key rotation tests ────────────────────────────────

#[test]
fn key_rotation_propose_and_accept() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let r = recipients(&env);
    let client = GovernanceContractClient::new(&env, &id);
    client.initialize(
        &admin,
        &String::from_str(&env, "StellarSwipe Gov"),
        &String::from_str(&env, "SSG"),
        &7u32,
        &SUPPLY,
        &r,
    );

    // Propose a key rotation.
    client.propose_key_rotation(&admin, &new_admin);

    // New admin accepts the rotation.
    client.accept_key_rotation(&new_admin);

    // Admin should now be the new admin.
    let h = client.health_check();
    assert_eq!(h.admin, new_admin);
}

#[test]
fn key_rotation_cancel() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let r = recipients(&env);
    let client = GovernanceContractClient::new(&env, &id);
    client.initialize(
        &admin,
        &String::from_str(&env, "StellarSwipe Gov"),
        &String::from_str(&env, "SSG"),
        &7u32,
        &SUPPLY,
        &r,
    );

    client.propose_key_rotation(&admin, &new_admin);
    client.cancel_key_rotation(&admin);

    // Accepting should fail since rotation was cancelled.
    let result = client.try_accept_key_rotation(&new_admin);
    assert!(result.is_err());
}

#[test]
fn emergency_revoke_admin() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let r = recipients(&env);
    let client = GovernanceContractClient::new(&env, &id);
    client.initialize(
        &admin,
        &String::from_str(&env, "StellarSwipe Gov"),
        &String::from_str(&env, "SSG"),
        &7u32,
        &SUPPLY,
        &r,
    );

    // Set the guardian.
    client.set_guardian(&admin, &guardian);

    // Emergency revoke removes admin access.
    client.emergency_revoke_admin(&guardian);

    // Health check should report uninitialized.
    let h = client.health_check();
    assert!(!h.is_initialized);
}
