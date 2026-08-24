#![cfg(test)]

use crate::categories::{RiskLevel, SignalCategory};
use crate::types::SignalAction;
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

#[test]
fn ai_oracle_can_set_score() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let cid = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &cid);

    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);
    client.set_ai_oracle(&admin, &oracle);

    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Alpha"),
        &(env.ledger().timestamp() + 86_400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    client.set_ai_score(&oracle, &signal_id, &72u32);

    let s = client.get_signal(&signal_id).unwrap();
    assert_eq!(s.ai_validation_score, Some(72u32));
}

#[test]
fn unauthorized_cannot_set_ai_score() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let cid = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &cid);

    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let attacker = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);
    client.set_ai_oracle(&admin, &oracle);

    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Alpha"),
        &(env.ledger().timestamp() + 86_400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    assert!(client
        .try_set_ai_score(&attacker, &signal_id, &50u32)
        .is_err());
}

#[test]
fn signal_without_score_still_readable() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let cid = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &cid);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "No AI"),
        &(env.ledger().timestamp() + 86_400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Low,
    );

    let s = client.get_signal(&signal_id).unwrap();
    assert_eq!(s.ai_validation_score, None);
}

#[test]
fn score_above_100_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let cid = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &cid);

    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);
    client.set_ai_oracle(&admin, &oracle);

    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Alpha"),
        &(env.ledger().timestamp() + 86_400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    assert!(client
        .try_set_ai_score(&oracle, &signal_id, &101u32)
        .is_err());
}
