#![cfg(test)]

use super::BridgeContract;
use crate::BridgeContractClient;
use crate::governance::{
    initialize_bridge, initialize_bridge_governance, Bridge, BridgeSecurityConfig, BridgeStatus,
    GovernanceDataKey,
};
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

fn signers(env: &Env, count: u32) -> Vec<Address> {
    let mut v = Vec::new(env);
    let mut i = 0u32;
    while i < count {
        v.push_back(Address::generate(env));
        i += 1;
    }
    v
}

fn security_config(_env: &Env) -> BridgeSecurityConfig {
    BridgeSecurityConfig {
        max_transfer_amount: 1_000_000_000,
        daily_transfer_limit: 10_000_000_000,
        min_validator_signatures: 2,
        transfer_delay_seconds: 300,
    }
}

#[test]
fn health_uninitialized() {
    let env = Env::default();
    let id = env.register_contract(None, BridgeContract);
    let client = BridgeContractClient::new(&env, &id);
    let h = client.health_check();
    assert!(!h.is_initialized);
    assert!(!h.is_paused);
}

#[test]
fn health_initialized_running() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, BridgeContract);
    let gov = signers(&env, 5);
    let validators = signers(&env, 3);
    let sec = security_config(&env);

    env.as_contract(&id, || {
        initialize_bridge_governance(&env, 1, gov.clone(), 3).expect("gov");
        initialize_bridge(&env, 1, validators.clone(), 2, sec).expect("bridge");
    });

    let client = BridgeContractClient::new(&env, &id);
    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(!h.is_paused);
    assert_eq!(h.admin, validators.get(0).expect("validator"));
}

#[test]
fn health_initialized_paused() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register_contract(None, BridgeContract);
    let gov = signers(&env, 5);
    let validators = signers(&env, 3);
    let sec = security_config(&env);

    env.as_contract(&id, || {
        initialize_bridge_governance(&env, 1, gov, 3).expect("gov");
        initialize_bridge(&env, 1, validators.clone(), 2, sec).expect("bridge");
        let mut bridge: Bridge = env
            .storage()
            .persistent()
            .get(&GovernanceDataKey::Bridge(1))
            .expect("bridge stored");
        bridge.status = BridgeStatus::Paused;
        env.storage()
            .persistent()
            .set(&GovernanceDataKey::Bridge(1), &bridge);
    });

    let client = BridgeContractClient::new(&env, &id);
    let h = client.health_check();
    assert!(h.is_initialized);
    assert!(h.is_paused);
}
