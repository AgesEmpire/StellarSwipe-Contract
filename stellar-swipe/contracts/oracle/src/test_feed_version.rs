#![cfg(test)]

//! Tests for Issue #1306: monotonic, observable feed configuration versions.

use super::*;
use feeds::FeedConfigInput;
use soroban_sdk::{testutils::{Address as _, Events}, Address, Env, String, Symbol, TryFromVal};

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

fn entry(env: &Env, p: &AssetPair, owner: &Address, max_age: u64) -> FeedConfigInput {
    FeedConfigInput {
        pair: p.clone(),
        owner: owner.clone(),
        publishers: Vec::new(env),
        max_age,
        enabled: true,
        max_movement_bps: 0,
        min_price: 1,
        max_price: i128::MAX,
    }
}

/// Versions emitted via `feed_cfg` during the last top-level invocation.
fn emitted_versions(env: &Env) -> Vec<u32> {
    let mut versions = Vec::new(env);
    for version in env
        .events()
        .all()
        .iter()
        .filter_map(|(_, topics, data)| {
            let name = topics.get(1)?;
            if Symbol::try_from_val(env, &name).ok()? != Symbol::new(env, "feed_cfg") {
                return None;
            }
            let values: Vec<soroban_sdk::Val> = soroban_sdk::IntoVal::into_val(&data, env);
            let version: u32 = TryFromVal::try_from_val(env, &values.get(1)?).ok()?;
            Some(version)
        })
    {
        versions.push_back(version);
    }
    versions
}

#[test]
fn configuration_changes_increment_version_and_emit_it() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    let publisher = Address::generate(&env);

    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 300)]));
    assert_eq!(emitted_versions(&env), Vec::from_array(&env, [1u32]));
    assert_eq!(client.get_feed_config(&p).unwrap().version, 1);

    client.add_feed_publisher(&owner, &p, &publisher);
    assert_eq!(emitted_versions(&env), Vec::from_array(&env, [2u32]));
    assert_eq!(client.get_feed_config(&p).unwrap().version, 2);

    client.revoke_feed_publisher(&owner, &p, &publisher);
    assert_eq!(emitted_versions(&env), Vec::from_array(&env, [3u32]));
    assert_eq!(client.get_feed_config(&p).unwrap().version, 3);

    client.configure_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 600)]));
    assert_eq!(emitted_versions(&env), Vec::from_array(&env, [4u32]));
    assert_eq!(client.get_feed_config(&p).unwrap().version, 4);
}

#[test]
fn failed_configuration_does_not_change_version_or_emit() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 300)]));

    assert_eq!(
        client.try_configure_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 0)])),
        Err(Ok(OracleError::InvalidFeedConfig))
    );
    assert_eq!(client.get_feed_config(&p).unwrap().version, 1);
    assert!(emitted_versions(&env).is_empty());

    // A non-owner cannot mutate the feed either.
    let stranger = Address::generate(&env);
    assert_eq!(
        client.try_add_feed_publisher(&stranger, &p, &stranger),
        Err(Ok(OracleError::Unauthorized))
    );
    assert_eq!(client.get_feed_config(&p).unwrap().version, 1);
    assert!(emitted_versions(&env).is_empty());
}

#[test]
fn price_read_exposes_configuration_version() {
    let env = Env::default();
    let (client, admin, p) = setup(&env);
    let owner = Address::generate(&env);
    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, &p, &owner, 300)]));
    client.set_price(&p, &1_000_000);
    assert_eq!(
        client.get_price_with_feed_version(&p),
        (1_000_000, client.get_feed_config(&p).unwrap().version)
    );

    client.add_feed_publisher(&owner, &p, &Address::generate(&env));
    assert_eq!(client.get_price_with_feed_version(&p), (1_000_000, 2));
}
