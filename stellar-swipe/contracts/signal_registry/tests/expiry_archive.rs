//! Signal expiration and archive behavior (issue #1219).
//!
//! Covers the expiry boundary, query visibility before and after cleanup,
//! bounded cursor-driven cleanup and archival, and retry idempotency. The
//! rules under test are specified in `docs/signal_expiration.md`.

use signal_registry::{
    RiskLevel, SignalAction, SignalCategory, SignalExpiryState, SignalRegistry,
    SignalRegistryClient, SortOption,
};
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger},
    vec, Address, Env, String, Symbol, TryFromVal,
};

const T0: u64 = 1_000_000;
const THIRTY_DAYS: u64 = 30 * 24 * 60 * 60;

fn setup() -> (Env, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(T0);
    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);
    client.initialize(&Address::generate(&env));
    (env, client)
}

/// Create a signal from a fresh, staked provider so rate limits and
/// per-provider caps never interfere. Returns the signal id.
fn create(env: &Env, client: &SignalRegistryClient, expiry: u64) -> u64 {
    let provider = Address::generate(env);
    client.stake_tokens(&provider, &1_000_000_000i128);
    client.create_signal(
        &provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Medium,
    )
}

fn active_ids(client: &SignalRegistryClient) -> std::vec::Vec<u64> {
    client
        .get_active_signals(&0, &100, &SortOption::RecencyDesc, &None, &None)
        .iter()
        .map(|s| s.id)
        .collect()
}

fn count_events(env: &Env, name: &str) -> usize {
    let wanted = Symbol::new(env, name);
    env.events()
        .all()
        .iter()
        .filter(|(_, topics, _)| {
            topics
                .get(0)
                .and_then(|t| Symbol::try_from_val(env, &t).ok())
                .map(|s| s == wanted)
                .unwrap_or(false)
        })
        .count()
}

fn set_time(env: &Env, t: u64) {
    env.ledger().set_timestamp(t);
}

#[test]
fn signal_is_live_through_its_expiry_second() {
    let (env, client) = setup();
    let id = create(&env, &client, T0 + 10);

    set_time(&env, T0 + 10);
    assert_eq!(client.get_signal_expiry_state(&id), SignalExpiryState::Live);
    assert_eq!(active_ids(&client), [id]);
    assert_eq!(
        client
            .get_active_signals_archived(&Address::generate(&env), &false)
            .len(),
        1
    );

    // Cleanup at the boundary must not expire it either.
    let (_, expired) = client.cleanup_expired_signals(&0);
    assert_eq!(expired, 0);
    assert_eq!(client.get_signal_expiry_state(&id), SignalExpiryState::Live);
}

#[test]
fn expired_signal_is_hidden_before_cleanup_runs() {
    let (env, client) = setup();
    let id = create(&env, &client, T0 + 10);

    set_time(&env, T0 + 11);
    // No cleanup yet: status is still Active in storage, but queries hide it.
    assert_eq!(
        client.get_signal_expiry_state(&id),
        SignalExpiryState::ExpiredPendingCleanup
    );
    assert!(active_ids(&client).is_empty());
    assert_eq!(
        client
            .get_active_signals_archived(&Address::generate(&env), &false)
            .len(),
        0
    );
    assert_eq!(client.get_pending_expiry_count(), 1);

    let (processed, expired) = client.cleanup_expired_signals(&0);
    assert_eq!((processed, expired), (1, 1));
    assert_eq!(
        client.get_signal_expiry_state(&id),
        SignalExpiryState::Expired
    );
    assert!(active_ids(&client).is_empty());
    assert_eq!(client.get_pending_expiry_count(), 0);
}

#[test]
fn cleanup_is_bounded_and_resumes_past_long_lived_signals() {
    let (env, client) = setup();
    // Two long-lived signals occupy the lowest ids; before the cursor, a small
    // budget was spent on them on every call and ids 3..=5 were never reached.
    let long_a = create(&env, &client, T0 + 20 * 24 * 60 * 60);
    let long_b = create(&env, &client, T0 + 20 * 24 * 60 * 60);
    let short: std::vec::Vec<u64> = (0..3).map(|_| create(&env, &client, T0 + 10)).collect();
    set_time(&env, T0 + 100);

    assert_eq!(client.cleanup_expired_signals(&2), (2, 0)); // ids 1, 2
    assert_eq!(client.cleanup_expired_signals(&2), (2, 2)); // ids 3, 4
    assert_eq!(client.cleanup_expired_signals(&2), (2, 1)); // id 5, wraps to 1
    assert_eq!(client.get_pending_expiry_count(), 0);

    for id in &short {
        assert_eq!(
            client.get_signal_expiry_state(id),
            SignalExpiryState::Expired
        );
    }
    for id in [long_a, long_b] {
        assert_eq!(client.get_signal_expiry_state(&id), SignalExpiryState::Live);
    }
    let mut live = active_ids(&client);
    live.sort();
    assert_eq!(live, [long_a, long_b]);
}

#[test]
fn cleanup_budget_is_clamped() {
    let (env, client) = setup();
    for _ in 0..3 {
        create(&env, &client, T0 + 10);
    }
    set_time(&env, T0 + 100);
    // A budget larger than the map examines each entry once, never twice.
    assert_eq!(client.cleanup_expired_signals(&1_000), (3, 3));
}

#[test]
fn cleanup_retries_are_idempotent() {
    let (env, client) = setup();
    for _ in 0..3 {
        create(&env, &client, T0 + 10);
    }
    set_time(&env, T0 + 100);

    assert_eq!(client.cleanup_expired_signals(&0), (3, 3));
    assert_eq!(count_events(&env, "signal_expired"), 3);

    // Retrying the full sweep changes nothing and announces nothing new.
    for _ in 0..3 {
        assert_eq!(client.cleanup_expired_signals(&0), (3, 0));
        assert_eq!(count_events(&env, "signal_expired"), 0);
    }
    assert_eq!(client.get_expired_count(), 3);
}

#[test]
fn archive_waits_for_threshold_then_removes_even_without_cleanup() {
    let (env, client) = setup();
    let id = create(&env, &client, T0 + 10);

    // Exactly at the threshold: not yet archivable.
    set_time(&env, T0 + 10 + THIRTY_DAYS);
    assert_eq!(client.archive_old_signals(&0), 0);
    assert_eq!(
        client.get_signal_expiry_state(&id),
        SignalExpiryState::ExpiredPendingCleanup
    );

    // One second later it is archived, although cleanup never ran.
    set_time(&env, T0 + 11 + THIRTY_DAYS);
    assert_eq!(client.archive_old_signals(&0), 1);
    // `signal_expired` is still announced for the never-cleaned signal.
    assert_eq!(count_events(&env, "signal_expired"), 1);
    assert_eq!(count_events(&env, "signals_archived"), 1);

    assert_eq!(
        client.get_signal_expiry_state(&id),
        SignalExpiryState::Removed
    );
    assert!(client.get_signal(&id).is_none());
    assert!(active_ids(&client).is_empty());

    // Retrying finds nothing more to do.
    assert_eq!(client.archive_old_signals(&0), 0);
    assert_eq!(count_events(&env, "signals_archived"), 0);
}

#[test]
fn archive_is_bounded_and_resumable() {
    let (env, client) = setup();
    let live = create(&env, &client, T0 + 20 * 24 * 60 * 60);
    for _ in 0..3 {
        create(&env, &client, T0 + 10);
    }
    set_time(&env, T0 + 100);
    client.cleanup_expired_signals(&0);
    set_time(&env, T0 + 100 + THIRTY_DAYS);

    // Budget of 1 examines one entry per call: the live signal first, then
    // one archivable signal per call.
    assert_eq!(client.archive_old_signals(&1), 0);
    assert_eq!(client.archive_old_signals(&1), 1);
    assert_eq!(client.archive_old_signals(&1), 1);
    assert_eq!(client.archive_old_signals(&1), 1);
    assert_eq!(client.archive_old_signals(&1), 0);
    assert_eq!(client.get_expired_count(), 0);
    // Past its own expiry by now, but not archivable yet.
    assert_eq!(
        client.get_signal_expiry_state(&live),
        SignalExpiryState::ExpiredPendingCleanup
    );
}

#[test]
fn unknown_ids_are_distinguished_from_removed_ids() {
    let (env, client) = setup();
    create(&env, &client, T0 + 10);
    assert_eq!(
        client.get_signal_expiry_state(&0),
        SignalExpiryState::Unknown
    );
    assert_eq!(
        client.get_signal_expiry_state(&2),
        SignalExpiryState::Unknown
    );
}
