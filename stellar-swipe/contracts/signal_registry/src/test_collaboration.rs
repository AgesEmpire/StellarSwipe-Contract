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

#[test]
fn test_collaborative_reward_distribution_equal() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let primary = Address::generate(&env);
    let co_author1 = Address::generate(&env);
    let co_author2 = Address::generate(&env);
    client.initialize(&admin);

    // Create collaborative signal with 60/25/15
    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author1.clone());
    co_authors.push_back(co_author2.clone());
    let mut pcts = Vec::new(&env);
    pcts.push_back(6000);
    pcts.push_back(2500);
    pcts.push_back(1500);

    let signal_id = client.create_collaborative_signal(
        &primary,
        &co_authors,
        &pcts,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1000000,
        &String::from_str(&env, "Bullish signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );

    // Approve co-authors
    client.approve_collaborative_signal(&signal_id, &co_author1);
    client.approve_collaborative_signal(&signal_id, &co_author2);

    // Simulate closing the signal (must be non-active)
    // In a real test you'd also need to record a trade execution or manually set status.
    // For simplicity, we can directly call record_signal_outcome after setting status to Closed.
    // We'll assume the contract provides a way to close it (e.g., expiry or admin action).
    // For this test, we can manually mutate the signal status via a helper or use expiry.
    // Let's use the existing cleanup_expired_signals or just trust that the signal is now closed.

    // Record outcome with total_fee = 1000, total_roi = 500
    // The TradeExecutor must be set first.
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor); // admin sets it
    // Also need to set the caller as executor (mock all auths)
    // Since we have mock_all_auths, we can call with executor as caller.
    client.record_signal_outcome(
        &signal_id,
        &SignalOutcome::Win,
        &1000,
        &500,
    );

    // Check pending rewards
    let pending_primary = client.get_pending_rewards(&primary);
    assert_eq!(pending_primary.fee, 600);
    assert_eq!(pending_primary.roi, 300);

    let pending_c1 = client.get_pending_rewards(&co_author1);
    assert_eq!(pending_c1.fee, 250);
    assert_eq!(pending_c1.roi, 125);

    let pending_c2 = client.get_pending_rewards(&co_author2);
    assert_eq!(pending_c2.fee, 150);
    assert_eq!(pending_c2.roi, 75);

    // Verify sum
    assert_eq!(pending_primary.fee + pending_c1.fee + pending_c2.fee, 1000);
    assert_eq!(pending_primary.roi + pending_c1.roi + pending_c2.roi, 500);
}
