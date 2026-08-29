#![cfg(test)]

extern crate std;

use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::errors::AdminError;
use soroban_sdk::{
    testutils::Address as _, testutils::Ledger, vec, Address, Env, InvokeError, String,
};

fn setup(env: &Env) -> (Address, SignalRegistryClient<'_>) {
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 10_000);
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (admin, client)
}

fn create_signal(env: &Env, client: &SignalRegistryClient, provider: &Address) -> u64 {
    let expiry = env.ledger().timestamp() + 86_400;
    client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "Cooldown test rationale"),
        &expiry,
        &SignalCategory::SWING,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Medium,
    )
}

fn try_create_signal(
    env: &Env,
    client: &SignalRegistryClient,
    provider: &Address,
) -> Result<Result<u64, soroban_sdk::Error>, Result<AdminError, soroban_sdk::InvokeError>> {
    let expiry = env.ledger().timestamp() + 86_400;
    client.try_create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "Cooldown test rationale"),
        &expiry,
        &SignalCategory::SWING,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Medium,
    )
}

#[test]
fn default_cooldown_is_zero() {
    let env = Env::default();
    let (_, client) = setup(&env);
    assert_eq!(client.get_submission_cooldown(), 0u64);
}

#[test]
fn admin_sets_and_gets_cooldown() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    assert_eq!(client.get_submission_cooldown(), 3_600u64);
    client.set_submission_cooldown(&admin, &0u64);
    assert_eq!(client.get_submission_cooldown(), 0u64);
}

#[test]
fn no_cooldown_allows_consecutive_signals() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    // Default cooldown = 0 — consecutive signals at the same timestamp succeed
    create_signal(&env, &client, &provider);
    create_signal(&env, &client, &provider);
}

#[test]
fn first_signal_always_succeeds_regardless_of_cooldown() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    let provider = Address::generate(&env);
    // First submission — no recorded last_signal_time → should succeed
    create_signal(&env, &client, &provider);
}

#[test]
fn second_signal_blocked_within_cooldown_window() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    let provider = Address::generate(&env);
    create_signal(&env, &client, &provider);
    // Still within cooldown — attempt fails
    let result = try_create_signal(&env, &client, &provider);
    assert_eq!(result, Err(Ok(AdminError::CooldownNotElapsed)));
}

#[test]
fn second_signal_succeeds_exactly_at_cooldown_boundary() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    let provider = Address::generate(&env);
    let first_ts = env.ledger().timestamp();
    create_signal(&env, &client, &provider);
    // Advance to exactly last_ts + cooldown — should succeed (>=, not >)
    env.ledger().with_mut(|l| l.timestamp = first_ts + 3_600);
    create_signal(&env, &client, &provider);
}

#[test]
fn second_signal_blocked_one_second_before_boundary() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    let provider = Address::generate(&env);
    let first_ts = env.ledger().timestamp();
    create_signal(&env, &client, &provider);
    // One second short of the cooldown
    env.ledger().with_mut(|l| l.timestamp = first_ts + 3_599);
    let result = try_create_signal(&env, &client, &provider);
    assert_eq!(result, Err(Ok(AdminError::CooldownNotElapsed)));
}

#[test]
fn second_signal_succeeds_after_cooldown() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    let provider = Address::generate(&env);
    let first_ts = env.ledger().timestamp();
    create_signal(&env, &client, &provider);
    env.ledger().with_mut(|l| l.timestamp = first_ts + 3_601);
    create_signal(&env, &client, &provider);
}

#[test]
fn setting_cooldown_to_zero_re_enables_consecutive_submissions() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    let provider = Address::generate(&env);
    create_signal(&env, &client, &provider);
    // Disable cooldown
    client.set_submission_cooldown(&admin, &0u64);
    // Should succeed immediately without advancing the clock
    create_signal(&env, &client, &provider);
}

#[test]
fn cooldown_is_per_provider() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_submission_cooldown(&admin, &3_600u64);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    create_signal(&env, &client, &p1);
    // p2 has no prior submission — should succeed even though p1 is in cooldown
    create_signal(&env, &client, &p2);
    // p1 is still blocked
    let result = try_create_signal(&env, &client, &p1);
    assert_eq!(result, Err(Ok(AdminError::CooldownNotElapsed)));
}
