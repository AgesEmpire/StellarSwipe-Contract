#![cfg(test)]

//! Tests for Issue #1305: configuration-driven price-movement breaker.

use super::*;
use feeds::FeedConfigInput;
use soroban_sdk::{testutils::{Address as _, Events}, Address, Env, String, TryFromVal};

fn pair(env: &Env) -> AssetPair {
    AssetPair {
        base: Asset { code: String::from_str(env, "XLM"), issuer: None },
        quote: Asset { code: String::from_str(env, "USDC"), issuer: None },
    }
}

fn setup(env: &Env) -> (OracleContractClient<'_>, Address, AssetPair) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(env, &id);
    client.initialize(&admin, &Asset { code: String::from_str(env, "XLM"), issuer: None });
    let p = pair(env);
    (client, admin, p)
}

fn entry(env: &Env, p: &AssetPair, owner: &Address, bps: u32) -> FeedConfigInput {
    FeedConfigInput {
        pair: p.clone(),
        owner: owner.clone(),
        publishers: Vec::new(env),
        max_age: 300,
        enabled: true,
        max_movement_bps: bps,
        min_price: 1,
        max_price: i128::MAX,
    }
}

#[test]
fn configured_threshold_boundary_and_outlier_event() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 500)]));
    assert_eq!(client.get_feed_config(&p).unwrap().max_movement_bps, 500);
    assert_eq!(client.get_update_deviation_threshold(&p), 500);

    client.set_price(&p, &1_000_000);
    // Exactly 5% is accepted.
    client.set_price(&p, &1_050_000);
    assert_eq!(client.get_price(&p), 1_050_000);

    // Just beyond 5% trips the breaker and is not stored. The test env clears
    // events per top-level call, so inspect them immediately after the call.
    client.set_price(&p, &1_200_000);
    let trip_emitted = env
        .events()
        .all()
        .iter()
        .filter_map(|(_, topics, _)| topics.get(1))
        .filter_map(|t| soroban_sdk::Symbol::try_from_val(&env, &t).ok())
        .any(|n| n == soroban_sdk::Symbol::new(&env, "dev_trip"));
    assert!(trip_emitted, "outlier must emit a dev_trip diagnostic event");
    assert!(client.is_update_dev_breaker_tripped(&p));
    assert_eq!(client.try_get_price(&p), Err(Ok(OracleError::PriceDeviationBreakerTripped)));
    assert_eq!(client.get_feed_status(&p).latest_round, 2);
    client.reset_update_deviation_breaker(&admin, &p);
    assert_eq!(client.get_price(&p), 1_050_000);
}

#[test]
fn repeated_outliers_reset_recovery_and_admin_recovery_works() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 500)]));
    client.set_price(&p, &1_000_000);
    client.set_price(&p, &2_000_000); // trips
    assert!(client.is_update_dev_breaker_tripped(&p));

    // A deviating update restarts the recovery streak.
    for _ in 0..2 {
        client.set_price(&p, &1_010_000);
    }
    assert_eq!(client.get_feed_status(&p).recovery_streak, 2);
    client.set_price(&p, &2_000_000);
    assert_eq!(client.get_feed_status(&p).recovery_streak, 0);
    assert!(client.is_update_dev_breaker_tripped(&p));

    // Documented recovery: authorized admin reset, then normal updates resume.
    client.reset_update_deviation_breaker(&admin, &p);
    assert!(!client.is_update_dev_breaker_tripped(&p));
    client.set_price(&p, &1_000_000);
    assert_eq!(client.get_price(&p), 1_000_000);
}

#[test]
fn invalid_threshold_is_rejected() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    assert_eq!(
        client.try_register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 10_001)])),
        Err(Ok(OracleError::InvalidFeedConfig))
    );
    assert!(client.get_feed_config(&p).is_none());
}
