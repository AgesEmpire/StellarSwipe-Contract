#![cfg(test)]

use crate::{staleness::OracleStatus, OracleContract, OracleContractClient};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    Address, Env, String, TryFromVal,
};
use stellar_swipe_common::emergency::CAT_ALL;
use stellar_swipe_common::{Asset, AssetPair};

fn xlm(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    }
}

fn usdc(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "USDC"),
        issuer: Some(Address::generate(env)),
    }
}

fn pair(env: &Env) -> AssetPair {
    AssetPair {
        base: usdc(env),
        quote: xlm(env),
    }
}

#[test]
fn health_not_initialized_without_admin() {
    let env = Env::default();
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
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
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
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
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
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

#[test]
fn heartbeat_is_healthy_after_recent_update() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let asset_pair = pair(&env);
    client.initialize(&admin, &xlm(&env));

    client.set_price(&asset_pair, &1_000);
    let health = client.check_oracle_heartbeat(&asset_pair);

    assert!(health.is_healthy);
    assert_eq!(health.status, OracleStatus::Healthy);
    assert_eq!(health.ledgers_since_update, 0);
}

#[test]
fn heartbeat_becomes_stale_after_price_age_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let asset_pair = pair(&env);
    client.initialize(&admin, &xlm(&env));
    client.set_price(&asset_pair, &1_000);

    env.ledger().with_mut(|ledger| ledger.sequence_number += 61);
    let health = client.check_oracle_heartbeat(&asset_pair);

    assert!(!health.is_healthy);
    assert_eq!(health.status, OracleStatus::Stale);
    assert_eq!(health.ledgers_since_update, 61);
}

#[test]
fn heartbeat_becomes_dead_after_dead_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let asset_pair = pair(&env);
    client.initialize(&admin, &xlm(&env));
    client.set_price(&asset_pair, &1_000);

    env.ledger()
        .with_mut(|ledger| ledger.sequence_number += 1_441);
    let health = client.check_oracle_heartbeat(&asset_pair);

    assert!(!health.is_healthy);
    assert_eq!(health.status, OracleStatus::Dead);
    assert_eq!(health.ledgers_since_update, 1_441);
}

fn heartbeat_event_count(env: &Env) -> usize {
    env.events()
        .all()
        .iter()
        .filter(|event| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = event.1.clone();
            let first = topics
                .get(0)
                .and_then(|val| soroban_sdk::Symbol::try_from_val(env, &val).ok());
            let second = topics
                .get(1)
                .and_then(|val| soroban_sdk::Symbol::try_from_val(env, &val).ok());
            first == Some(soroban_sdk::symbol_short!("oracle"))
                && second == Some(soroban_sdk::symbol_short!("hb_missed"))
        })
        .count()
}

#[test]
fn heartbeat_missed_event_emits_when_becoming_stale() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let asset_pair = pair(&env);
    client.initialize(&admin, &xlm(&env));
    client.set_price(&asset_pair, &1_000);

    env.ledger().with_mut(|ledger| ledger.sequence_number += 61);
    assert_eq!(
        client.check_oracle_heartbeat(&asset_pair).status,
        OracleStatus::Stale
    );
    assert_eq!(heartbeat_event_count(&env), 1);
}

#[test]
fn heartbeat_missed_event_emits_when_transitioning_to_dead() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let asset_pair = pair(&env);
    client.initialize(&admin, &xlm(&env));
    client.set_price(&asset_pair, &1_000);

    env.ledger().with_mut(|ledger| ledger.sequence_number += 61);
    assert_eq!(
        client.check_oracle_heartbeat(&asset_pair).status,
        OracleStatus::Stale
    );

    env.ledger()
        .with_mut(|ledger| ledger.sequence_number += 1_380);
    assert_eq!(
        client.check_oracle_heartbeat(&asset_pair).status,
        OracleStatus::Dead
    );
    assert_eq!(heartbeat_event_count(&env), 1);
}
