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
