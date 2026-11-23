//! Integration-level tests for Issue #1422: category-specific signal submission quotas.
#![cfg(test)]
use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::submission_quota::{CategoryQuotaConfig, QuotaConfig};
use soroban_sdk::{testutils::{Address as _, Ledger}, vec, Address, Env, String};

fn setup(env: &Env) -> (Address, SignalRegistryClient) {
    env.mock_all_auths();
    env.ledger().set_timestamp(10_000);
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (admin, client)
}

fn make_signal(
    env: &Env,
    client: &SignalRegistryClient,
    provider: &Address,
    category: SignalCategory,
) -> Result<u64, crate::errors::AdminError> {
    client
        .try_create_signal(
            provider,
            &String::from_str(env, "XLM/USDC"),
            &crate::types::SignalAction::Buy,
            &100,
            &String::from_str(env, "test rationale"),
            &(env.ledger().timestamp() + 3600),
            &category,
            &vec![env, String::from_str(env, "test")],
            &RiskLevel::Medium,
        )
        .map_err(|e| e.unwrap())
}

// ── category quota does not bypass global limit ───────────────────────────────

#[test]
fn test_category_quota_does_not_bypass_global_limit() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);

    // Global quota: max 1 submission per provider per window.
    client.set_submission_quota(
        &admin,
        &QuotaConfig {
            window_ledgers: 1000,
            per_provider_limit: 1,
            global_limit: 10000,
        },
    );
    // Category quota: generous ceiling.
    client.set_category_submission_quota(
        &admin,
        &SignalCategory::SWING,
        &CategoryQuotaConfig {
            window_ledgers: 1000,
            category_limit: 50,
        },
    );

    // First submission succeeds.
    make_signal(&env, &client, &provider, SignalCategory::SWING).unwrap();

    // Second is blocked by the global per-provider cap.
    let result = make_signal(&env, &client, &provider, SignalCategory::SWING);
    assert!(result.is_err(), "global per-provider cap must still apply");
}

// ── category quota exhaustion ─────────────────────────────────────────────────

#[test]
fn test_category_quota_exhaustion_blocks_submission() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);

    // Category SCALP: max 2 total per window.
    client.set_category_submission_quota(
        &admin,
        &SignalCategory::SCALP,
        &CategoryQuotaConfig {
            window_ledgers: 1000,
            category_limit: 2,
        },
    );

    make_signal(&env, &client, &p1, SignalCategory::SCALP).unwrap();
    make_signal(&env, &client, &p2, SignalCategory::SCALP).unwrap();

    // Third SCALP submission (from a fresh provider) must be blocked.
    let p3 = Address::generate(&env);
    let result = make_signal(&env, &client, &p3, SignalCategory::SCALP);
    assert!(result.is_err(), "category ceiling must block third SCALP submission");
}

// ── category switching ────────────────────────────────────────────────────────

#[test]
fn test_category_quota_does_not_affect_other_categories() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);

    // Set SCALP quota to 1.
    client.set_category_submission_quota(
        &admin,
        &SignalCategory::SCALP,
        &CategoryQuotaConfig {
            window_ledgers: 1000,
            category_limit: 1,
        },
    );

    make_signal(&env, &client, &provider, SignalCategory::SCALP).unwrap();
    // SCALP exhausted.
    assert!(make_signal(&env, &client, &provider, SignalCategory::SCALP).is_err());

    // SWING has no quota — different provider can still submit.
    let p2 = Address::generate(&env);
    let result = make_signal(&env, &client, &p2, SignalCategory::SWING);
    assert!(result.is_ok(), "SWING quota must be independent of SCALP quota");
}

// ── window reset ──────────────────────────────────────────────────────────────

#[test]
fn test_category_quota_resets_at_window_boundary() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);

    client.set_category_submission_quota(
        &admin,
        &SignalCategory::ARBITRAGE,
        &CategoryQuotaConfig {
            window_ledgers: 10,
            category_limit: 1,
        },
    );

    make_signal(&env, &client, &provider, SignalCategory::ARBITRAGE).unwrap();
    assert!(make_signal(&env, &client, &provider, SignalCategory::ARBITRAGE).is_err());

    // Advance past window boundary.
    env.ledger().with_mut(|li| li.sequence_number += 10);

    let p2 = Address::generate(&env);
    let result = make_signal(&env, &client, &p2, SignalCategory::ARBITRAGE);
    assert!(result.is_ok(), "category quota must reset after window boundary");
}

// ── counters update atomically with successful submissions ────────────────────

#[test]
fn test_category_usage_increments_atomically() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    client.set_category_submission_quota(
        &admin,
        &SignalCategory::SWING,
        &CategoryQuotaConfig {
            window_ledgers: 1000,
            category_limit: 10,
        },
    );

    assert_eq!(client.get_category_submission_usage(&SignalCategory::SWING), 0);

    let p1 = Address::generate(&env);
    make_signal(&env, &client, &p1, SignalCategory::SWING).unwrap();
    assert_eq!(client.get_category_submission_usage(&SignalCategory::SWING), 1);

    let p2 = Address::generate(&env);
    make_signal(&env, &client, &p2, SignalCategory::SWING).unwrap();
    assert_eq!(client.get_category_submission_usage(&SignalCategory::SWING), 2);
}
