#![cfg(test)]

//! Tests for Issues #1309 (publisher authorization), #1310 (breaker
//! hysteresis), #1311 (feed health) and #1312 (atomic batch config).

use super::*;
use feeds::{FeedConfigInput, FeedState, MAX_FEED_BATCH};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String,
};

fn asset(env: &Env, code: &str) -> Asset {
    Asset {
        code: String::from_str(env, code),
        issuer: None,
    }
}

fn pair(env: &Env, base: &str) -> AssetPair {
    AssetPair {
        base: asset(env, base),
        quote: asset(env, "USDC"),
    }
}

fn setup(env: &Env) -> (OracleContractClient<'_>, Address) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let admin = Address::generate(env);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(env, &id);
    client.initialize(&admin, &asset(env, "XLM"));
    (client, admin)
}

fn entry(env: &Env, p: AssetPair, owner: &Address, max_age: u64) -> FeedConfigInput {
    FeedConfigInput {
        pair: p,
        owner: owner.clone(),
        publishers: Vec::new(env),
        max_age,
        enabled: true,
    }
}

fn next_ledger(env: &Env) {
    env.ledger().with_mut(|l| l.sequence_number += 1);
}

// ── #1309 ────────────────────────────────────────────────────────────────────

#[test]
fn publisher_authorization_owner_delegate_revoked_unrelated() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let p = pair(&env, "XLM");
    let owner = Address::generate(&env);
    let delegate = Address::generate(&env);
    let stranger = Address::generate(&env);
    for a in [&owner, &delegate, &stranger] {
        client.add_price_source(&admin, a, &1);
    }
    client.configure_feeds(
        &admin,
        &Vec::from_array(&env, [entry(&env, p.clone(), &owner, 300)]),
    );

    // Owner may submit.
    client.submit_pair_price(&owner, &p, &100, &100);
    assert_eq!(client.get_feed_status(&p).latest_round, 1);

    // Unrelated caller is rejected and state is unchanged.
    assert_eq!(
        client.try_submit_pair_price(&stranger, &p, &100, &100),
        Err(Ok(OracleError::Unauthorized))
    );
    assert_eq!(client.get_feed_status(&p).latest_round, 1);

    // Only the owner may delegate.
    assert_eq!(
        client.try_add_feed_publisher(&stranger, &p, &delegate),
        Err(Ok(OracleError::Unauthorized))
    );
    client.add_feed_publisher(&owner, &p, &delegate);
    client.submit_pair_price(&delegate, &p, &100, &100);
    assert_eq!(client.get_feed_status(&p).latest_round, 2);

    // Revoked delegate is rejected.
    client.revoke_feed_publisher(&owner, &p, &delegate);
    next_ledger(&env);
    assert_eq!(
        client.try_submit_pair_price(&delegate, &p, &100, &100),
        Err(Ok(OracleError::Unauthorized))
    );
    assert_eq!(client.get_feed_status(&p).latest_round, 2);
    assert_eq!(client.get_feed_config(&p).unwrap().version, 3);
}

// ── #1310 ────────────────────────────────────────────────────────────────────

fn tripped(env: &Env) -> (OracleContractClient<'_>, Address, AssetPair) {
    let (client, admin) = setup(env);
    let p = pair(env, "XLM");
    client.set_update_deviation_threshold(&admin, &p, &500);
    client.set_price(&p, &1_000_000);
    client.set_price(&p, &2_000_000); // trips
    assert!(client.is_update_dev_breaker_tripped(&p));
    (client, admin, p)
}

#[test]
fn partial_recovery_does_not_clear_breaker() {
    let env = Env::default();
    let (client, _, p) = tripped(&env);
    for _ in 0..price_cb::RECOVERY_ROUNDS - 1 {
        client.set_price(&p, &1_010_000);
    }
    assert!(client.is_update_dev_breaker_tripped(&p));
    assert!(client.try_get_price(&p).is_err());

    // A marginal deviating observation restarts the streak.
    client.set_price(&p, &1_200_000);
    assert_eq!(client.get_feed_status(&p).recovery_streak, 0);
    client.set_price(&p, &1_010_000);
    assert!(client.is_update_dev_breaker_tripped(&p));
}

#[test]
fn consecutive_valid_rounds_resume_aggregation() {
    let env = Env::default();
    let (client, _, p) = tripped(&env);
    for _ in 0..price_cb::RECOVERY_ROUNDS - 1 {
        client.set_price(&p, &1_010_000);
    }
    client.set_price(&p, &1_020_000);
    assert!(!client.is_update_dev_breaker_tripped(&p));
    assert_eq!(client.get_price(&p), 1_020_000);
}

#[test]
fn authorized_reset_clears_streak() {
    let env = Env::default();
    let (client, admin, p) = tripped(&env);
    client.set_price(&p, &1_010_000);
    client.reset_update_deviation_breaker(&admin, &p);
    let s = client.get_feed_status(&p);
    assert!(!s.breaker_tripped);
    assert_eq!(s.recovery_streak, 0);
    client.set_price(&p, &1_000_000);
    assert_eq!(client.get_price(&p), 1_000_000);
}

#[test]
fn halted_feed_rejects_aggregation_submissions() {
    let env = Env::default();
    let (client, admin, p) = tripped(&env);
    let src = Address::generate(&env);
    client.add_price_source(&admin, &src, &1);
    assert_eq!(
        client.try_submit_pair_price(&src, &p, &1_000_000, &100),
        Err(Ok(OracleError::PriceDeviationBreakerTripped))
    );
}

// ── #1311 ────────────────────────────────────────────────────────────────────

#[test]
fn health_reports_unknown_active_stale_disabled() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let p = pair(&env, "XLM");
    let owner = Address::generate(&env);

    assert_eq!(client.get_feed_status(&p).state, FeedState::Unknown);

    client.configure_feeds(
        &admin,
        &Vec::from_array(&env, [entry(&env, p.clone(), &owner, 60)]),
    );
    let s = client.get_feed_status(&p);
    assert_eq!(s.state, FeedState::Stale);
    assert!(s.configured && s.stale);

    client.set_price(&p, &100);
    let s = client.get_feed_status(&p);
    assert_eq!(s.state, FeedState::Active);
    assert_eq!((s.latest_round, s.last_update), (1, 1_000));

    // Freshness boundary: age 59 is active, age 60 is stale.
    env.ledger().set_timestamp(1_059);
    assert_eq!(client.get_feed_status(&p).state, FeedState::Active);
    env.ledger().set_timestamp(1_060);
    assert_eq!(client.get_feed_status(&p).state, FeedState::Stale);

    let mut e = entry(&env, p.clone(), &owner, 60);
    e.enabled = false;
    client.configure_feeds(&admin, &Vec::from_array(&env, [e]));
    let s = client.get_feed_status(&p);
    assert_eq!(s.state, FeedState::Disabled);
    assert_eq!(s.config_version, 2);
    assert_eq!(
        client.try_submit_pair_price(&owner, &p, &100, &100),
        Err(Ok(OracleError::FeedDisabled))
    );
}

// ── #1312 ────────────────────────────────────────────────────────────────────

#[test]
fn valid_batch_applies_all_entries() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    let (a, b) = (pair(&env, "XLM"), pair(&env, "BTC"));
    client.configure_feeds(
        &admin,
        &Vec::from_array(
            &env,
            [
                entry(&env, a.clone(), &owner, 60),
                entry(&env, b.clone(), &owner, 90),
            ],
        ),
    );
    assert_eq!(client.get_feed_config(&a).unwrap().version, 1);
    assert_eq!(client.get_feed_config(&b).unwrap().max_age, 90);
}

#[test]
fn invalid_middle_entry_leaves_everything_unchanged() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    let (a, b, c) = (pair(&env, "XLM"), pair(&env, "BTC"), pair(&env, "ETH"));
    client.configure_feeds(
        &admin,
        &Vec::from_array(&env, [entry(&env, a.clone(), &owner, 60)]),
    );

    let batch = Vec::from_array(
        &env,
        [
            entry(&env, a.clone(), &owner, 120),
            entry(&env, b.clone(), &owner, 0), // invalid
            entry(&env, c.clone(), &owner, 60),
        ],
    );
    assert_eq!(
        client.try_configure_feeds(&admin, &batch),
        Err(Ok(OracleError::InvalidFeedConfig))
    );
    let cfg = client.get_feed_config(&a).unwrap();
    assert_eq!((cfg.version, cfg.max_age), (1, 60));
    assert!(client.get_feed_config(&b).is_none());
    assert!(client.get_feed_config(&c).is_none());
}

#[test]
fn duplicate_entries_are_rejected() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    let a = pair(&env, "XLM");
    let batch = Vec::from_array(
        &env,
        [
            entry(&env, a.clone(), &owner, 60),
            entry(&env, a.clone(), &owner, 90),
        ],
    );
    assert_eq!(
        client.try_configure_feeds(&admin, &batch),
        Err(Ok(OracleError::InvalidFeedConfig))
    );
    assert!(client.get_feed_config(&a).is_none());

    let mut dup = entry(&env, a.clone(), &owner, 60);
    dup.publishers = Vec::from_array(&env, [owner.clone(), owner.clone()]);
    assert_eq!(
        client.try_configure_feeds(&admin, &Vec::from_array(&env, [dup])),
        Err(Ok(OracleError::InvalidFeedConfig))
    );
}

#[test]
fn batch_size_is_capped() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    let codes = [
        "A0", "A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A9", "B0",
    ];
    let mut batch = Vec::new(&env);
    for code in codes.iter().take(MAX_FEED_BATCH as usize) {
        batch.push_back(entry(&env, pair(&env, code), &owner, 60));
    }
    client.configure_feeds(&admin, &batch);

    batch.push_back(entry(&env, pair(&env, codes[10]), &owner, 60));
    assert_eq!(
        client.try_configure_feeds(&admin, &batch),
        Err(Ok(OracleError::InvalidFeedBatch))
    );
    assert!(client.get_feed_config(&pair(&env, codes[10])).is_none());
    assert_eq!(
        client.try_configure_feeds(&admin, &Vec::new(&env)),
        Err(Ok(OracleError::InvalidFeedBatch))
    );
}
