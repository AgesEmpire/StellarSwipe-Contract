#![cfg(test)]

use crate::categories::{RiskLevel, SignalCategory};
use crate::types::{SignalAction, SignalOutcome};
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};
use stellar_swipe_common::rate_limit::ActionType;

fn setup() -> (Env, Address, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(50_000);
    #[allow(deprecated)]
    let id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &id);
    let admin = Address::generate(&env);
    let executor = Address::generate(&env);
    client.initialize(&admin);
    client.set_trade_executor(&admin, &executor);
    client.set_tier_signal_limits(&admin, &64, &64, &64);
    client.set_rate_limit_config(&admin, &ActionType::SignalSubmission, &3_600, &64);
    (env, admin, executor, client)
}

fn create_closed_signal(
    env: &Env,
    client: &SignalRegistryClient,
    provider: &Address,
    index: u32,
) -> u64 {
    let expiry = env.ledger().timestamp() + 1;
    let signal_id = client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &(100_000 + index as i128),
        &String::from_str(env, "Reward stress signal rationale"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(env),
        &RiskLevel::Medium,
    );
    env.ledger().set_timestamp(expiry + 1);
    client.cleanup_expired_signals(&128);
    signal_id
}

#[test]
fn stress_many_reward_events_claim_once_per_provider() {
    let (env, _, executor, client) = setup();
    let provider = Address::generate(&env);
    let mut expected_fee = 0i128;
    let mut expected_roi = 0i128;

    for i in 0..32 {
        let signal_id = create_closed_signal(&env, &client, &provider, i);
        let fee = 100 + i as i128;
        let roi = 50 + i as i128;
        client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Profit, &fee, &roi);
        expected_fee += fee;
        expected_roi += roi;
    }

    assert_eq!(
        client.claim_pending_rewards(&provider),
        (expected_fee, expected_roi)
    );
    assert_eq!(client.claim_pending_rewards(&provider), (0, 0));
}

#[test]
fn stress_many_providers_claims_preserve_reward_totals() {
    let (env, _, executor, client) = setup();
    let mut providers = Vec::new(&env);
    let mut expected_fee_total = 0i128;
    let mut expected_roi_total = 0i128;

    for i in 0..24 {
        let provider = Address::generate(&env);
        let signal_id = create_closed_signal(&env, &client, &provider, i);
        let fee = 1_000 + (i as i128 * 3);
        let roi = 500 + (i as i128 * 2);
        client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Profit, &fee, &roi);
        providers.push_back(provider);
        expected_fee_total += fee;
        expected_roi_total += roi;
    }

    let mut claimed_fee_total = 0i128;
    let mut claimed_roi_total = 0i128;
    for provider in providers.iter() {
        let (fee, roi) = client.claim_pending_rewards(&provider);
        claimed_fee_total += fee;
        claimed_roi_total += roi;
        assert_eq!(client.claim_pending_rewards(&provider), (0, 0));
    }

    assert_eq!(claimed_fee_total, expected_fee_total);
    assert_eq!(claimed_roi_total, expected_roi_total);
}
