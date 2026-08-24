#![cfg(test)]
use crate::types::SignalOutcome;
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

fn setup() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, admin, client)
}

/// Creates a collaborative signal, has every co-author approve it, then
/// advances past expiry and runs cleanup so the signal leaves `Active` and
/// `record_signal_outcome` can be called.
fn create_approved_and_closed_signal(
    env: &Env,
    client: &SignalRegistryClient,
    primary: &Address,
    co_authors: &Vec<Address>,
    contribution_pcts: &Vec<u32>,
) -> u64 {
    let expiry = env.ledger().timestamp() + 86400;
    let signal_id = client.create_collaborative_signal(
        primary,
        co_authors,
        contribution_pcts,
        &String::from_str(env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "Bullish signal"),
        &expiry,
    );

    for co_author in co_authors.iter() {
        client.approve_collaborative_signal(&signal_id, &co_author);
    }

    env.ledger().set_timestamp(expiry + 1);
    client.cleanup_expired_signals(&100);
    signal_id
}

#[test]
fn test_create_collaborative_signal() {
    let (env, _admin, client) = setup();
    let primary = Address::generate(&env);
    let co_author1 = Address::generate(&env);
    let co_author2 = Address::generate(&env);

    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author1);
    co_authors.push_back(co_author2);

    let mut contribution_pcts = Vec::new(&env);
    contribution_pcts.push_back(6000); // Primary: 60%
    contribution_pcts.push_back(2500); // Co-author1: 25%
    contribution_pcts.push_back(1500); // Co-author2: 15%

    let expiry = env.ledger().timestamp() + 86400;
    let signal_id = client.create_collaborative_signal(
        &primary,
        &co_authors,
        &contribution_pcts,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Bullish signal"),
        &expiry,
    );

    assert!(signal_id > 0);
    assert!(client.is_collaborative_signal(&signal_id));
}

#[test]
fn test_approve_collaborative_signal() {
    let (env, _admin, client) = setup();
    let primary = Address::generate(&env);
    let co_author = Address::generate(&env);

    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author.clone());

    let mut contribution_pcts = Vec::new(&env);
    contribution_pcts.push_back(6000);
    contribution_pcts.push_back(4000);

    let expiry = env.ledger().timestamp() + 86400;
    let signal_id = client.create_collaborative_signal(
        &primary,
        &co_authors,
        &contribution_pcts,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Bullish signal"),
        &expiry,
    );

    client.approve_collaborative_signal(&signal_id, &co_author);

    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.status, crate::types::SignalStatus::Active);
}

#[test]
fn test_invalid_contribution_percentages() {
    let (env, _admin, client) = setup();
    let primary = Address::generate(&env);
    let co_author = Address::generate(&env);

    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author);

    let mut contribution_pcts = Vec::new(&env);
    contribution_pcts.push_back(6000);
    contribution_pcts.push_back(3000); // Total = 9000, not 10000

    let expiry = env.ledger().timestamp() + 86400;
    let result = client.try_create_collaborative_signal(
        &primary,
        &co_authors,
        &contribution_pcts,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Bullish signal"),
        &expiry,
    );
    assert!(result.is_err());
}

#[test]
fn test_reward_distribution_single_contributor() {
    let (env, admin, client) = setup();
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

    let primary = Address::generate(&env);
    let co_authors = Vec::new(&env);
    let mut pcts = Vec::new(&env);
    pcts.push_back(10000); // Primary: 100%

    let signal_id = create_approved_and_closed_signal(&env, &client, &primary, &co_authors, &pcts);

    client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Profit, &1000, &500);

    assert_eq!(client.claim_pending_rewards(&primary), (1000, 500));
}

#[test]
fn test_reward_distribution_two_equal_contributors() {
    let (env, admin, client) = setup();
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

    let primary = Address::generate(&env);
    let co_author = Address::generate(&env);
    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author.clone());
    let mut pcts = Vec::new(&env);
    pcts.push_back(5000);
    pcts.push_back(5000);

    let signal_id = create_approved_and_closed_signal(&env, &client, &primary, &co_authors, &pcts);

    client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Profit, &1000, &500);

    assert_eq!(client.claim_pending_rewards(&primary), (500, 250));
    assert_eq!(client.claim_pending_rewards(&co_author), (500, 250));
}

#[test]
fn test_reward_distribution_three_unequal_contributors() {
    let (env, admin, client) = setup();
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

    let primary = Address::generate(&env);
    let co_author1 = Address::generate(&env);
    let co_author2 = Address::generate(&env);
    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author1.clone());
    co_authors.push_back(co_author2.clone());
    let mut pcts = Vec::new(&env);
    pcts.push_back(6000); // Primary: 60%
    pcts.push_back(2500); // Co-author1: 25%
    pcts.push_back(1500); // Co-author2: 15%

    let signal_id = create_approved_and_closed_signal(&env, &client, &primary, &co_authors, &pcts);

    client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Profit, &1000, &500);

    let (primary_fee, primary_roi) = client.claim_pending_rewards(&primary);
    let (c1_fee, c1_roi) = client.claim_pending_rewards(&co_author1);
    let (c2_fee, c2_roi) = client.claim_pending_rewards(&co_author2);

    assert_eq!((primary_fee, primary_roi), (600, 300));
    assert_eq!((c1_fee, c1_roi), (250, 125));
    assert_eq!((c2_fee, c2_roi), (150, 75));

    // No dust loss: shares must sum back to the totals passed in.
    assert_eq!(primary_fee + c1_fee + c2_fee, 1000);
    assert_eq!(primary_roi + c1_roi + c2_roi, 500);
}

#[test]
fn test_reward_distribution_rounding_remainder_goes_to_last_contributor() {
    let (env, admin, client) = setup();
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

    let primary = Address::generate(&env);
    let co_author1 = Address::generate(&env);
    let co_author2 = Address::generate(&env);
    let mut co_authors = Vec::new(&env);
    co_authors.push_back(co_author1.clone());
    co_authors.push_back(co_author2.clone());
    let mut pcts = Vec::new(&env);
    // 1/3 splits do not divide evenly into an indivisible fee amount.
    pcts.push_back(3334);
    pcts.push_back(3333);
    pcts.push_back(3333);

    let signal_id = create_approved_and_closed_signal(&env, &client, &primary, &co_authors, &pcts);

    // total_fee = 100 is not evenly divisible by these basis-point shares.
    client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Profit, &100, &10);

    let (primary_fee, primary_roi) = client.claim_pending_rewards(&primary);
    let (c1_fee, c1_roi) = client.claim_pending_rewards(&co_author1);
    let (c2_fee, c2_roi) = client.claim_pending_rewards(&co_author2);

    // Floor division: 100 * 3334 / 10000 = 33, 100 * 3333 / 10000 = 33 (x2).
    // The last contributor absorbs the 1-unit remainder so the sum is exact.
    assert_eq!(primary_fee, 33);
    assert_eq!(c1_fee, 33);
    assert_eq!(c2_fee, 34);
    assert_eq!(primary_fee + c1_fee + c2_fee, 100);
    assert_eq!(primary_roi + c1_roi + c2_roi, 10);
}
