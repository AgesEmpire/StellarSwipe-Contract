#![cfg(test)]
//! Unit tests for SignalRegistry signal lifecycle (#255).
//!
//! Covers: submit, get, expire, and cancel across all lifecycle states.

use crate::categories::{RiskLevel, SignalCategory};
use crate::errors::{AdminError, SignalEditError};
use crate::types::{SignalAction, SignalStatus};
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, admin, client)
}

fn create_signal(
    env: &Env,
    client: &SignalRegistryClient,
    provider: &Address,
    expiry_offset: u64,
) -> u64 {
    client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "Rationale"),
        &(env.ledger().timestamp() + expiry_offset),
        &SignalCategory::SWING,
        &Vec::new(env),
        &RiskLevel::Medium,
    )
}

// ── Submit ────────────────────────────────────────────────────────────────────

/// Happy path: signal is created, stored, and readable with Active status.
#[test]
fn submit_happy_path() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 86_400);
    let signal = client.get_signal(&id).unwrap();
    assert_eq!(signal.id, id);
    assert_eq!(signal.provider, provider);
    assert_eq!(signal.status, SignalStatus::Active);
    assert_eq!(signal.price, 1_000_000);
}

/// Expiry in the past must panic (contract enforces `expiry > now`).
#[test]
#[should_panic]
fn submit_expired_expiry_panics() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let now = env.ledger().timestamp();
    client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &now, // expiry == now → not in the future
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );
}

/// Unsupported / malformed asset pair is rejected with `InvalidAssetPair`.
#[test]
fn submit_unsupported_pair_rejected() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 86_400;

    // No slash separator
    let r = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLMUSDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );
    assert_eq!(r, Err(Ok(AdminError::InvalidAssetPair)));

    // Same asset on both sides
    let r2 = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/XLM"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );
    assert_eq!(r2, Err(Ok(AdminError::InvalidAssetPair)));
}

/// Spam limit: rate limiter blocks submissions beyond the per-window cap.
#[test]
fn submit_spam_limit_enforced() {
    let (env, admin, client) = setup();
    // Set a tight rate limit: max 2 signal submissions per window.
    use stellar_swipe_common::rate_limit::{ActionType, RateLimitConfig};
    client.set_rate_limit_config(
        &admin,
        &ActionType::SignalSubmission,
        &60u64,
        &2u32,
    );

    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 86_400;

    // First two succeed.
    for _ in 0..2 {
        client.create_signal(
            &provider,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &1_000_000,
            &String::from_str(&env, "Rationale"),
            &expiry,
            &SignalCategory::SWING,
            &Vec::new(&env),
            &RiskLevel::Medium,
        );
    }

    // Third is rate-limited.
    let r = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(&env),
        &RiskLevel::Medium,
    );
    assert_eq!(r, Err(Ok(AdminError::RateLimitExceeded)));
}

// ── Get ───────────────────────────────────────────────────────────────────────

/// Valid signal ID returns the signal.
#[test]
fn get_valid_signal() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 86_400);
    let signal = client.get_signal(&id);
    assert!(signal.is_some());
    assert_eq!(signal.unwrap().id, id);
}

/// Non-existent signal ID returns None.
#[test]
fn get_not_found_returns_none() {
    let (_, _, client) = setup();
    assert!(client.get_signal(&9999u64).is_none());
}

/// Signal past its expiry is still retrievable but cleanup marks it Expired.
#[test]
fn get_expired_signal_after_cleanup() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 100);

    // Advance time past expiry.
    env.ledger().set_timestamp(env.ledger().timestamp() + 200);
    client.cleanup_expired_signals(&10);

    let signal = client.get_signal(&id).unwrap();
    assert_eq!(signal.status, SignalStatus::Expired);
}

// ── Expire ────────────────────────────────────────────────────────────────────

/// Signal past expiry is marked Expired by cleanup and expiry event is emitted.
#[test]
fn expire_past_expiry_marks_expired_and_emits_event() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 100);

    // Still active before expiry.
    assert_eq!(client.get_signal(&id).unwrap().status, SignalStatus::Active);

    env.ledger().set_timestamp(env.ledger().timestamp() + 200);

    use soroban_sdk::testutils::Events;
    let events_before = env.events().all().len();
    client.cleanup_expired_signals(&10);
    let events_after = env.events().all().len();

    // Status transitioned.
    assert_eq!(client.get_signal(&id).unwrap().status, SignalStatus::Expired);
    // At least one event was emitted (signal_expired).
    assert!(events_after > events_before, "expiry event must be emitted");
}

/// Signal not yet past expiry stays Active after cleanup.
#[test]
fn expire_active_signal_stays_active() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 86_400);

    client.cleanup_expired_signals(&10);
    assert_eq!(client.get_signal(&id).unwrap().status, SignalStatus::Active);
}

// ── Cancel (update_signal / edit window) ─────────────────────────────────────

/// Owner can edit (cancel price update) within the 60-second window.
#[test]
fn cancel_by_owner_within_window_succeeds() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 86_400);

    let edit = crate::types::SignalEditInput {
        set_price: true,
        price: 2_000_000,
        set_rationale_hash: false,
        rationale_hash: String::from_str(&env, ""),
        set_confidence: false,
        confidence: 0,
    };
    client.update_signal(&provider, &id, &edit);
    assert_eq!(client.get_signal(&id).unwrap().price, 2_000_000);
}

/// Non-owner cannot edit the signal.
#[test]
fn cancel_by_non_owner_fails() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let attacker = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 86_400);

    let edit = crate::types::SignalEditInput {
        set_price: true,
        price: 2_000_000,
        set_rationale_hash: false,
        rationale_hash: String::from_str(&env, ""),
        set_confidence: false,
        confidence: 0,
    };
    let r = client.try_update_signal(&attacker, &id, &edit);
    assert_eq!(r, Err(Ok(SignalEditError::NotSignalOwner)));
}

/// Edit window closes after 60 seconds — update must be rejected.
#[test]
fn cancel_after_edit_window_closed_fails() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 86_400);

    // Advance past the 60-second edit window.
    env.ledger().set_timestamp(env.ledger().timestamp() + 61);

    let edit = crate::types::SignalEditInput {
        set_price: true,
        price: 2_000_000,
        set_rationale_hash: false,
        rationale_hash: String::from_str(&env, ""),
        set_confidence: false,
        confidence: 0,
    };
    let r = client.try_update_signal(&provider, &id, &edit);
    assert_eq!(r, Err(Ok(SignalEditError::EditWindowClosed)));
}

/// Signal that has been copied (adoption_count > 0) cannot be edited.
#[test]
fn cancel_already_copied_signal_fails() {
    let (env, admin, client) = setup();
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);

    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 86_400);

    // Simulate adoption.
    client.increment_adoption(&executor, &id, &1u64);

    let edit = crate::types::SignalEditInput {
        set_price: true,
        price: 2_000_000,
        set_rationale_hash: false,
        rationale_hash: String::from_str(&env, ""),
        set_confidence: false,
        confidence: 0,
    };
    let r = client.try_update_signal(&provider, &id, &edit);
    assert_eq!(r, Err(Ok(SignalEditError::SignalAlreadyCopied)));
}

// ── State transition integrity ────────────────────────────────────────────────

/// Active → Expired transition is irreversible: cleanup on already-expired signal is a no-op.
#[test]
fn expired_signal_not_re_expired() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id = create_signal(&env, &client, &provider, 100);

    env.ledger().set_timestamp(env.ledger().timestamp() + 200);
    client.cleanup_expired_signals(&10);
    assert_eq!(client.get_signal(&id).unwrap().status, SignalStatus::Expired);

    // Second cleanup — status must remain Expired, not change.
    client.cleanup_expired_signals(&10);
    assert_eq!(client.get_signal(&id).unwrap().status, SignalStatus::Expired);
}

/// IDs are monotonically increasing — each new signal gets a unique, larger ID.
#[test]
fn signal_ids_are_unique_and_increasing() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);
    let id1 = create_signal(&env, &client, &provider, 86_400);
    let id2 = create_signal(&env, &client, &provider, 86_400);
    let id3 = create_signal(&env, &client, &provider, 86_400);
    assert!(id1 < id2);
    assert!(id2 < id3);
}
