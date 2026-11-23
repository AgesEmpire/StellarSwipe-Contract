//! Integration-level tests for Issue #1420: appeal window for provider suspension decisions.
//!
//! These tests exercise the full contract entrypoints so the authorization,
//! status transitions and event emissions all go through the real code paths.
#![cfg(test)]
use super::*;
use crate::provider_status::ProviderStatus;
use crate::suspension_appeal::{AppealOutcome, AppealPolicy, AppealState};
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env};

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

fn suspend_provider(admin: &Address, provider: &Address, client: &SignalRegistryClient) {
    // Active → Suspended.
    client.set_provider_status(admin, provider, &ProviderStatus::Suspended);
}

// ── eligibility ───────────────────────────────────────────────────────────────

#[test]
fn test_active_provider_not_eligible_for_appeal() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let provider = Address::generate(&env);
    // Default status is Active.
    assert!(!client.is_appeal_eligible(&provider));
}

#[test]
fn test_suspended_provider_is_eligible() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);
    assert!(client.is_appeal_eligible(&provider));
}

// ── submission ────────────────────────────────────────────────────────────────

#[test]
fn test_submit_appeal_creates_pending_record() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);

    let appeal = client.submit_suspension_appeal(&provider);
    assert_eq!(appeal.state, AppealState::Pending);
    assert_eq!(appeal.outcome, None);
    assert!(appeal.deadline > appeal.submitted_at);
}

#[test]
fn test_duplicate_appeal_fails() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);

    client.submit_suspension_appeal(&provider);
    let result = client.try_submit_suspension_appeal(&provider);
    assert!(result.is_err(), "duplicate appeal must fail");
}

#[test]
fn test_appeal_deadline_respects_policy() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);

    client.set_appeal_policy(
        &admin,
        &AppealPolicy { window_secs: 7200 },
    );

    let appeal = client.submit_suspension_appeal(&provider);
    assert_eq!(
        appeal.deadline,
        env.ledger().timestamp() + 7200,
        "deadline must equal submission_time + window_secs"
    );
}

// ── resolution ────────────────────────────────────────────────────────────────

#[test]
fn test_approve_appeal_restores_active() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);

    client.submit_suspension_appeal(&provider);
    client.resolve_suspension_appeal(&admin, &provider, &AppealOutcome::Approved);

    assert_eq!(client.get_provider_status(&provider), ProviderStatus::Active);
    let record = client.get_suspension_appeal(&provider).unwrap();
    assert_eq!(record.state, AppealState::Resolved);
    assert_eq!(record.outcome, Some(AppealOutcome::Approved));
}

#[test]
fn test_deny_appeal_bans_provider() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);

    client.submit_suspension_appeal(&provider);
    client.resolve_suspension_appeal(&admin, &provider, &AppealOutcome::Denied);

    assert_eq!(client.get_provider_status(&provider), ProviderStatus::Banned);
    let record = client.get_suspension_appeal(&provider).unwrap();
    assert_eq!(record.outcome, Some(AppealOutcome::Denied));
}

#[test]
fn test_resolve_twice_fails() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);

    client.submit_suspension_appeal(&provider);
    client.resolve_suspension_appeal(&admin, &provider, &AppealOutcome::Approved);

    let result = client.try_resolve_suspension_appeal(&admin, &provider, &AppealOutcome::Denied);
    assert!(result.is_err(), "resolving an already-resolved appeal must fail");
}

#[test]
fn test_late_resolution_fails_with_pause_expired() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);

    client.set_appeal_policy(&admin, &AppealPolicy { window_secs: 3600 });
    client.submit_suspension_appeal(&provider);

    // Advance past deadline.
    env.ledger().with_mut(|li| li.timestamp += 3601);

    let result = client.try_resolve_suspension_appeal(&admin, &provider, &AppealOutcome::Approved);
    assert!(result.is_err(), "late resolution must return an error");

    // Appeal should be auto-denied on the late attempt.
    let record = client.get_suspension_appeal(&provider).unwrap();
    assert_eq!(record.state, AppealState::Resolved);
    assert_eq!(record.outcome, Some(AppealOutcome::Denied));
}

#[test]
fn test_resolve_no_record_fails() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);
    suspend_provider(&admin, &provider, &client);
    // No appeal submitted.
    let result = client.try_resolve_suspension_appeal(&admin, &provider, &AppealOutcome::Approved);
    assert!(result.is_err());
}
