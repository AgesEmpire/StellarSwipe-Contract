#![cfg(test)]

//! Tests for Issue #1304: per-asset oracle price bounds.

use super::*;
use feeds::FeedConfigInput;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn pair(env: &Env) -> AssetPair {
    AssetPair {
        base: Asset {
            code: String::from_str(env, "XLM"),
            issuer: None,
        },
        quote: Asset {
            code: String::from_str(env, "USDC"),
            issuer: None,
        },
    }
}

fn setup(env: &Env) -> (OracleContractClient<'_>, Address, AssetPair) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(env, &id);
    client.initialize(&admin, &Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    });
    let p = pair(env);
    (client, admin, p)
}

fn entry(env: &Env, p: &AssetPair, owner: &Address, min: i128, max: i128) -> FeedConfigInput {
    FeedConfigInput {
        pair: p.clone(),
        owner: owner.clone(),
        publishers: Vec::new(env),
        max_age: 300,
        enabled: true,
        max_movement_bps: 0,
        min_price: min,
        max_price: max,
    }
}

#[test]
fn exact_bounds_accepted_and_out_of_range_rejected() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 100, 200)]));

    client.set_price(&p, &100);
    assert_eq!(client.get_price(&p), 100);
    client.set_price(&p, &200);
    assert_eq!(client.get_price(&p), 200);

    assert_eq!(client.try_set_price(&p, &99), Err(Ok(OracleError::PriceOutOfBounds)));
    assert_eq!(client.try_set_price(&p, &201), Err(Ok(OracleError::PriceOutOfBounds)));
    // Rejected observations never replace the latest accepted price.
    assert_eq!(client.get_price(&p), 200);
    assert_eq!(client.get_feed_status(&p).latest_round, 2);
}

#[test]
fn submitted_observations_respect_bounds() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    client.add_price_source(&admin, &owner, &1);
    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 100, 200)]));

    assert_eq!(
        client.try_submit_pair_price(&owner, &p, &50, &100),
        Err(Ok(OracleError::PriceOutOfBounds))
    );
    client.submit_pair_price(&owner, &p, &150, &100);
    assert_eq!(client.get_price(&p), 150);
}

#[test]
fn invalid_bounds_are_rejected_atomically() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    for (min, max) in [(0, 200), (100, 100), (200, 100), (-1, 200)] {
        assert_eq!(
            client.try_register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, min, max)])),
            Err(Ok(OracleError::InvalidFeedConfig))
        );
    }
    assert!(client.get_feed_config(&p).is_none());
}

#[test]
fn bounds_update_changes_accepted_range() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 100, 200)]));
    client.set_price(&p, &150);

    let mut updated = entry(&env, &p, &owner, 150, 300);
    updated.enabled = true;
    client.configure_feeds(&admin, &Vec::from_array(&env, [updated]));
    assert_eq!(client.try_set_price(&p, &100), Err(Ok(OracleError::PriceOutOfBounds)));
    client.set_price(&p, &300);
    assert_eq!(client.get_price(&p), 300);
    assert_eq!(client.get_feed_config(&p).unwrap().min_price, 150);
    assert_eq!(client.get_feed_config(&p).unwrap().max_price, 300);
}
