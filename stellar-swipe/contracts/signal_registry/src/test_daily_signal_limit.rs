//! Tests for per-provider daily signal creation rate limit (issue #778)
//! and comprehensive input validation (issue #634).

#![cfg(test)]

extern crate std;

use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::errors::AdminError;
use soroban_sdk::{testutils::Address as _, testutils::Ledger, vec, Address, Env, String};

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
        &String::from_str(env, "Daily limit test rationale"),
        &expiry,
        &SignalCategory::SWING,
        &vec![env],
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
        &String::from_str(env, "Daily limit test rationale"),
        &expiry,
        &SignalCategory::SWING,
        &vec![env],
        &RiskLevel::Medium,
    )
}

// ── Issue #778: daily signal limit ──────────────────────────────────────────

#[test]
fn default_daily_limit_is_zero() {
    let env = Env::default();
    let (_, client) = setup(&env);
    assert_eq!(client.get_daily_signal_limit(), 0u32);
}

#[test]
fn admin_sets_and_gets_daily_limit() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_daily_signal_limit(&admin, &5u32);
    assert_eq!(client.get_daily_signal_limit(), 5u32);
    client.set_daily_signal_limit(&admin, &0u32);
    assert_eq!(client.get_daily_signal_limit(), 0u32);
}

#[test]
fn no_daily_limit_allows_many_signals() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    // Default limit = 0 (disabled) — multiple signals succeed
    create_signal(&env, &client, &provider);
    create_signal(&env, &client, &provider);
    create_signal(&env, &client, &provider);
}

#[test]
fn first_signal_always_succeeds_with_limit_set() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_daily_signal_limit(&admin, &1u32);
    let provider = Address::generate(&env);
    create_signal(&env, &client, &provider);
}

#[test]
fn second_signal_blocked_when_limit_is_one() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_daily_signal_limit(&admin, &1u32);
    let provider = Address::generate(&env);
    create_signal(&env, &client, &provider);
    let result = try_create_signal(&env, &client, &provider);
    assert_eq!(result, Err(Ok(AdminError::SignalLimitExceeded)));
}

#[test]
fn limit_allows_exactly_n_signals_per_day() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_daily_signal_limit(&admin, &3u32);
    let provider = Address::generate(&env);
    create_signal(&env, &client, &provider);
    create_signal(&env, &client, &provider);
    create_signal(&env, &client, &provider);
    let result = try_create_signal(&env, &client, &provider);
    assert_eq!(result, Err(Ok(AdminError::SignalLimitExceeded)));
}

#[test]
fn daily_limit_resets_on_new_day() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_daily_signal_limit(&admin, &1u32);
    let provider = Address::generate(&env);
    let start_ts = env.ledger().timestamp();
    create_signal(&env, &client, &provider);
    // Advance to the next day bucket
    env.ledger().with_mut(|l| l.timestamp = start_ts + 86_400);
    // Should succeed on the new day
    create_signal(&env, &client, &provider);
}

#[test]
fn daily_limit_is_per_provider() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_daily_signal_limit(&admin, &1u32);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    create_signal(&env, &client, &p1);
    // p2 has no prior submission — should succeed
    create_signal(&env, &client, &p2);
    // p1 is blocked
    let result = try_create_signal(&env, &client, &p1);
    assert_eq!(result, Err(Ok(AdminError::SignalLimitExceeded)));
}

#[test]
fn disabling_limit_allows_consecutive_signals() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    client.set_daily_signal_limit(&admin, &1u32);
    let provider = Address::generate(&env);
    create_signal(&env, &client, &provider);
    // Disable limit
    client.set_daily_signal_limit(&admin, &0u32);
    // Should succeed immediately
    create_signal(&env, &client, &provider);
}

// ── Issue #634: input validation ─────────────────────────────────────────────

#[test]
fn create_signal_rejects_zero_price() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 86_400;
    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &0i128,
        &String::from_str(&env, "rationale"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env],
        &RiskLevel::Medium,
    );
    assert_eq!(result, Err(Ok(AdminError::InvalidParameter)));
}

#[test]
fn create_signal_rejects_negative_price() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 86_400;
    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &-1i128,
        &String::from_str(&env, "rationale"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env],
        &RiskLevel::Medium,
    );
    assert_eq!(result, Err(Ok(AdminError::InvalidParameter)));
}

#[test]
fn create_signal_rejects_empty_rationale() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 86_400;
    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, ""),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env],
        &RiskLevel::Medium,
    );
    assert_eq!(result, Err(Ok(AdminError::InvalidParameter)));
}

#[test]
fn create_signal_rejects_past_expiry() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    // expiry == now (not strictly in the future)
    let expiry = env.ledger().timestamp();
    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, "rationale"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env],
        &RiskLevel::Medium,
    );
    assert_eq!(result, Err(Ok(AdminError::InvalidParameter)));
}
