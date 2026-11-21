#![cfg(test)]
//! Contract-level coverage for oracle observation integrity:
//! - Issue #1212: observations are identified by `(source, sequence)`; replays
//!   are rejected and sequence advancement supersedes rather than adds weight.
//! - Issue #1213: future-dated observations are rejected before storage/use.

use super::*;
use crate::types::ExternalPrice;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Bytes, Env, String,
};

const NOW: u64 = 10_000;
const SEQ: u32 = 500;

fn setup() -> (Env, OracleContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    env.ledger().set_sequence_number(SEQ);
    let id = env.register(OracleContract, ());
    let client = OracleContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(
        &admin,
        &Asset {
            code: String::from_str(&env, "XLM"),
            issuer: None,
        },
    );
    (env, client, admin)
}

fn pair(env: &Env) -> AssetPair {
    AssetPair {
        base: Asset {
            code: String::from_str(env, "USDC"),
            issuer: None,
        },
        quote: Asset {
            code: String::from_str(env, "XLM"),
            issuer: None,
        },
    }
}

fn advance_ledger(env: &Env) {
    env.ledger().with_mut(|l| {
        l.sequence_number += 1;
        l.timestamp += 5;
    });
}

// ── #1212: submit_price → calculate_consensus ────────────────────────────────

#[test]
fn consensus_replay_in_same_ledger_is_rejected() {
    let (env, client, admin) = setup();
    let o1 = Address::generate(&env);
    client.register_oracle(&admin, &o1);

    client.submit_price(&o1, &100_000_000);
    assert_eq!(
        client.try_submit_price(&o1, &100_000_000),
        Err(Ok(OracleError::DuplicateObservation))
    );
    // A different price at the same sequence is still the same observation id.
    assert_eq!(
        client.try_submit_price(&o1, &90_000_000),
        Err(Ok(OracleError::DuplicateObservation))
    );
}

#[test]
fn consensus_counts_each_source_once_across_sequence_advancement() {
    let (env, client, admin) = setup();
    let o1 = Address::generate(&env);
    let o2 = Address::generate(&env);
    let o3 = Address::generate(&env);
    client.register_oracle(&admin, &o1);
    client.register_oracle(&admin, &o2);
    client.register_oracle(&admin, &o3);

    // o1 floods observations over several ledgers; o2/o3 submit once.
    client.submit_price(&o1, &200_000_000);
    client.submit_price(&o2, &100_000_000);
    client.submit_price(&o3, &100_000_000);
    for _ in 0..4 {
        advance_ledger(&env);
        client.submit_price(&o1, &200_000_000);
    }

    // Without dedupe o1's five samples would pull the median to 200_000_000.
    assert_eq!(client.calculate_consensus(), 100_000_000);
    assert_eq!(client.get_consensus_price().unwrap().num_oracles, 3);
}

#[test]
fn consensus_sequence_advancement_uses_latest_observation() {
    let (env, client, admin) = setup();
    let o1 = Address::generate(&env);
    let o2 = Address::generate(&env);
    let o3 = Address::generate(&env);
    client.register_oracle(&admin, &o1);
    client.register_oracle(&admin, &o2);
    client.register_oracle(&admin, &o3);

    client.submit_price(&o1, &100_000_000);
    client.submit_price(&o2, &100_000_000);
    client.submit_price(&o3, &100_000_000);
    advance_ledger(&env);
    // o1 and o2 revise upward at the next sequence.
    client.submit_price(&o1, &104_000_000);
    client.submit_price(&o2, &104_000_000);

    assert_eq!(client.calculate_consensus(), 104_000_000);
    assert_eq!(client.get_consensus_price().unwrap().num_oracles, 3);
}

#[test]
fn consensus_distinct_sources_in_same_ledger_all_count() {
    let (env, client, admin) = setup();
    let o1 = Address::generate(&env);
    let o2 = Address::generate(&env);
    let o3 = Address::generate(&env);
    client.register_oracle(&admin, &o1);
    client.register_oracle(&admin, &o2);
    client.register_oracle(&admin, &o3);

    client.submit_price(&o1, &100_000_000);
    client.submit_price(&o2, &101_000_000);
    client.submit_price(&o3, &102_000_000);
    assert_eq!(client.calculate_consensus(), 101_000_000);
    assert_eq!(client.get_consensus_price().unwrap().num_oracles, 3);
}

// ── #1212: submit_pair_price → get_price_with_confidence ────────────────────

#[test]
fn pair_replay_cannot_satisfy_min_source_count() {
    let (env, client, admin) = setup();
    let o1 = Address::generate(&env);
    client.add_price_source(&admin, &o1, &1u32);
    client.set_min_source_count(&admin, &2u32);
    let pair = pair(&env);

    client.submit_pair_price(&o1, &pair, &1_000_000, &100u32);
    assert_eq!(
        client.try_submit_pair_price(&o1, &pair, &1_000_000, &100u32),
        Err(Ok(OracleError::DuplicateObservation))
    );

    // Advancing the sequence supersedes o1's observation; still one source.
    advance_ledger(&env);
    client.submit_pair_price(&o1, &pair, &1_010_000, &100u32);
    assert_eq!(
        client.try_get_price_with_confidence(&pair),
        Err(Ok(OracleError::InsufficientSources))
    );
}

#[test]
fn pair_distinct_sources_meet_min_source_count() {
    let (env, client, admin) = setup();
    let o1 = Address::generate(&env);
    let o2 = Address::generate(&env);
    client.add_price_source(&admin, &o1, &1u32);
    client.add_price_source(&admin, &o2, &1u32);
    client.set_min_source_count(&admin, &2u32);
    let pair = pair(&env);

    client.submit_pair_price(&o1, &pair, &1_000_000, &100u32);
    client.submit_pair_price(&o2, &pair, &1_000_000, &100u32);
    assert_eq!(client.get_price_with_confidence(&pair), (1_000_000, 100));
}

#[test]
fn pair_sequence_advancement_replaces_price() {
    let (env, client, admin) = setup();
    let o1 = Address::generate(&env);
    client.add_price_source(&admin, &o1, &1u32);
    let pair = pair(&env);

    client.submit_pair_price(&o1, &pair, &1_000_000, &100u32);
    advance_ledger(&env);
    client.submit_pair_price(&o1, &pair, &1_050_000, &90u32);
    assert_eq!(client.get_price_with_confidence(&pair), (1_050_000, 90));
}

// ── #1212 / #1213: update_with_external_data ────────────────────────────────

fn report(
    env: &Env,
    oracle: &Address,
    price: i128,
    round_id: u64,
    timestamp: u64,
) -> ExternalPrice {
    ExternalPrice {
        asset_pair: pair(env),
        price,
        timestamp,
        round_id,
        signature: Bytes::new(env),
        oracle_address: oracle.clone(),
        decimals: 7,
    }
}

#[test]
fn external_replayed_round_does_not_add_weight() {
    let (env, client, _) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let mut prices = Vec::new(&env);
    prices.push_back(report(&env, &a, 100, 1, NOW));
    prices.push_back(report(&env, &a, 100, 1, NOW));
    prices.push_back(report(&env, &a, 100, 1, NOW));
    prices.push_back(report(&env, &b, 200, 1, NOW));
    // Counted once each: (100 + 200) / 2.
    assert_eq!(client.update_with_external_data(&prices), 150);
}

#[test]
fn external_round_advancement_keeps_latest_round_only() {
    let (env, client, _) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let mut prices = Vec::new(&env);
    prices.push_back(report(&env, &a, 300, 2, NOW));
    prices.push_back(report(&env, &a, 100, 1, NOW)); // older round: ignored
    prices.push_back(report(&env, &b, 200, 5, NOW));
    prices.push_back(report(&env, &b, 400, 6, NOW)); // newer round: supersedes
    assert_eq!(client.update_with_external_data(&prices), 350);
}

#[test]
fn external_current_and_past_timestamps_are_accepted() {
    let (env, client, _) = setup();
    let a = Address::generate(&env);
    for ts in [NOW, NOW - 1] {
        let mut prices = Vec::new(&env);
        prices.push_back(report(&env, &a, 1_000, 1, ts));
        assert_eq!(client.update_with_external_data(&prices), 1_000);
    }
}

#[test]
fn external_future_timestamp_is_rejected_before_storage() {
    let (env, client, _) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let pair = pair(&env);

    for ts in [NOW + 1, u64::MAX] {
        let mut prices = Vec::new(&env);
        prices.push_back(report(&env, &a, 1_000, 1, NOW));
        prices.push_back(report(&env, &b, 5_000, 1, ts));
        assert_eq!(
            client.try_update_with_external_data(&prices),
            Err(Ok(OracleError::FutureTimestamp))
        );
    }
    // Nothing was stored for the pair.
    assert_eq!(
        client.try_get_price(&pair),
        Err(Ok(OracleError::PriceNotFound))
    );
}
