#![cfg(test)]
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use crate::categories::{RiskLevel, SignalCategory};
use crate::types::SignalAction;

#[test]
fn test_create_collaborative_signal() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let primary = Address::generate(&env);
    let co_author1 = Address::generate(&env);
    let co_author2 = Address::generate(&env);

    client.initialize(&admin);

    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author1.clone());
    co_authors.push_back(co_author2.clone());

    let mut contribution_pcts = Vec::new(&env);
    contribution_pcts.push_back(6000); // Primary: 60%
    contribution_pcts.push_back(2500); // Co-author1: 25%
    contribution_pcts.push_back(1500); // Co-author2: 15%

    let signal_id = client.create_collaborative_signal(
        &primary,
        &co_authors,
        &contribution_pcts,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1000000,
        &String::from_str(&env, "Bullish signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );

    assert!(signal_id > 0);
    assert!(client.is_collaborative_signal(&signal_id));
}

#[test]
fn test_approve_collaborative_signal() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let primary = Address::generate(&env);
    let co_author = Address::generate(&env);

    client.initialize(&admin);

    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author.clone());

    let mut contribution_pcts = Vec::new(&env);
    contribution_pcts.push_back(6000);
    contribution_pcts.push_back(4000);

    let signal_id = client.create_collaborative_signal(
        &primary,
        &co_authors,
        &contribution_pcts,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1000000,
        &String::from_str(&env, "Bullish signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );

    client.approve_collaborative_signal(&signal_id, &co_author);

    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.status, crate::types::SignalStatus::Active);
}

#[test]
#[should_panic(expected = "InvalidParameter")]
fn test_invalid_contribution_percentages() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let primary = Address::generate(&env);
    let co_author = Address::generate(&env);

    client.initialize(&admin);

    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author);

    let mut contribution_pcts = Vec::new(&env);
    contribution_pcts.push_back(6000);
    contribution_pcts.push_back(3000); // Total = 9000, not 10000

    client.create_collaborative_signal(
        &primary,
        &co_authors,
        &contribution_pcts,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1000000,
        &String::from_str(&env, "Bullish signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );
}
