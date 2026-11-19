#![cfg(test)]

//! Tests for Issue #1303: one canonical identifier per asset pair.

use super::*;
use feeds::FeedConfigInput;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn asset(env: &Env, code: &str) -> Asset {
    Asset {
        code: String::from_str(env, code),
        issuer: None,
    }
}

fn pair(env: &Env, base: &str, quote: &str) -> AssetPair {
    AssetPair {
        base: asset(env, base),
        quote: asset(env, quote),
    }
}

fn setup(env: &Env) -> (OracleContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(env, &id);
    client.initialize(&admin, &asset(env, "XLM"));
    (client, admin)
}

fn entry(env: &Env, p: AssetPair, owner: &Address) -> FeedConfigInput {
    FeedConfigInput {
        pair: p,
        owner: owner.clone(),
        publishers: Vec::new(env),
        max_age: 300,
        enabled: true,
        min_price: 1,
        max_price: i128::MAX,
    }
}

#[test]
fn reversed_pair_is_the_same_feed() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    let forward = pair(&env, "BTC", "USDC");
    let reverse = pair(&env, "USDC", "BTC");

    client.register_feeds(
        &admin,
        &Vec::from_array(&env, [entry(&env, forward.clone(), &owner)]),
    );
    // Reversed registration is rejected: the canonical identifier already exists.
    assert_eq!(
        client.try_register_feeds(&admin, &Vec::from_array(&env, [entry(&env, reverse.clone(), &owner)])),
        Err(Ok(OracleError::OracleAlreadyExists))
    );
    assert_eq!(client.get_canonical_pair(&forward), client.get_canonical_pair(&reverse));
    assert_eq!(client.get_feed_config(&reverse).unwrap().version, 1);
}

#[test]
fn aliases_collapse_to_one_identifier() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    let canonical = pair(&env, "XLM", "USDC");
    let alias = pair(&env, "xlm", "usdc");
    assert_eq!(client.get_canonical_pair(&canonical), client.get_canonical_pair(&alias));

    client.register_feeds(&admin, &Vec::from_array(&env, [entry(&env, canonical, &owner)]));
    assert_eq!(
        client.try_register_feeds(&admin, &Vec::from_array(&env, [entry(&env, alias, &owner)])),
        Err(Ok(OracleError::OracleAlreadyExists))
    );
}

#[test]
fn duplicate_pair_inside_a_batch_is_rejected() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    let batch = Vec::from_array(
        &env,
        [
            entry(&env, pair(&env, "XLM", "USDC"), &owner),
            entry(&env, pair(&env, "USDC", "xlm"), &owner),
        ],
    );
    assert_eq!(
        client.try_register_feeds(&admin, &batch),
        Err(Ok(OracleError::InvalidFeedConfig))
    );
    assert!(client.get_feed_config(&pair(&env, "XLM", "USDC")).is_none());
}

#[test]
fn independent_quote_assets_and_identical_assets() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let owner = Address::generate(&env);
    client.register_feeds(
        &admin,
        &Vec::from_array(
            &env,
            [
                entry(&env, pair(&env, "XLM", "USDC"), &owner),
                entry(&env, pair(&env, "XLM", "EURC"), &owner),
            ],
        ),
    );
    assert!(client.get_feed_config(&pair(&env, "XLM", "USDC")).is_some());
    assert!(client.get_feed_config(&pair(&env, "XLM", "EURC")).is_some());
    assert_eq!(
        client.try_get_canonical_pair(&pair(&env, "XLM", "xlm")),
        Err(Ok(OracleError::InvalidFeedConfig))
    );
}
