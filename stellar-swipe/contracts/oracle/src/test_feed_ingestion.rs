#![cfg(test)]
//! Contract-level coverage for feed ingestion:
//! - Issue #1405: decimal scale metadata and checked normalization at ingestion.
//! - Issue #1406: per-source heartbeat windows.
//! - Issue #1407: deterministic round identifiers exposed with the price.

use super::*;
use crate::storage::{normalize_price_checked, rescale_price};
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
    client.initialize(&admin, &asset(&env, "XLM"));
    (env, client, admin)
}

fn asset(env: &Env, code: &str) -> Asset {
    Asset {
        code: String::from_str(env, code),
        issuer: None,
    }
}

fn pair(env: &Env, base: &str) -> AssetPair {
    AssetPair {
        base: asset(env, base),
        quote: asset(env, "XLM"),
    }
}

fn report(
    env: &Env,
    pair: &AssetPair,
    oracle: &Address,
    price: i128,
    decimals: u32,
    round_id: u64,
    timestamp: u64,
) -> ExternalPrice {
    ExternalPrice {
        asset_pair: pair.clone(),
        price,
        timestamp,
        round_id,
        signature: Bytes::new(env),
        oracle_address: oracle.clone(),
        decimals,
    }
}

fn batch(env: &Env, reports: &[ExternalPrice]) -> Vec<ExternalPrice> {
    let mut v = Vec::new(env);
    for r in reports {
        v.push_back(r.clone());
    }
    v
}

fn at(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|l| {
        l.timestamp = timestamp;
        l.sequence_number += 1;
    });
}

// ── #1405: decimal normalization ─────────────────────────────────────────────

#[test]
fn feed_scale_metadata_defines_decimals_and_exponent() {
    let (env, client, admin) = setup();
    let p = pair(&env, "USDC");

    let scale = client.get_feed_scale(&p);
    assert_eq!(
        scale,
        FeedScale {
            decimals: CANONICAL_DECIMALS,
            exponent: -(CANONICAL_DECIMALS as i32),
            canonical_decimals: CANONICAL_DECIMALS,
            configured: false,
        }
    );

    client.set_feed_decimals(&admin, &p, &6);
    let scale = client.get_feed_scale(&p);
    assert_eq!((scale.decimals, scale.exponent, scale.configured), (6, -6, true));

    client.set_feed_decimals(&admin, &p, &MAX_FEED_DECIMALS);
    assert_eq!(client.get_feed_scale(&p).exponent, -(MAX_FEED_DECIMALS as i32));

    // Unsupported precision is rejected and leaves the metadata unchanged.
    assert_eq!(
        client.try_set_feed_decimals(&admin, &p, &(MAX_FEED_DECIMALS + 1)),
        Err(Ok(OracleError::UnsupportedPrecision))
    );
    assert_eq!(client.get_feed_decimals(&p), Some(MAX_FEED_DECIMALS));
}

#[test]
fn mixed_decimal_feeds_normalize_to_common_scale() {
    let (env, client, admin) = setup();
    let usdc = pair(&env, "USDC"); // 6 decimals
    let btc = pair(&env, "BTC"); // 8 decimals
    client.set_feed_decimals(&admin, &usdc, &6);
    client.set_feed_decimals(&admin, &btc, &8);

    // Both represent exactly 1.0 in their native scale.
    client.set_price(&usdc, &1_000_000);
    client.set_price(&btc, &100_000_000);

    assert_eq!(client.get_normalized_price(&usdc, &7), 10_000_000);
    assert_eq!(client.get_normalized_price(&btc, &7), 10_000_000);
    assert_eq!(client.get_normalized_price(&usdc, &18), 1_000_000_000_000_000_000);
    assert_eq!(
        client.try_get_normalized_price(&btc, &(MAX_FEED_DECIMALS + 1)),
        Err(Ok(OracleError::UnsupportedPrecision))
    );
}

#[test]
fn external_reports_with_mixed_decimals_are_normalized_at_ingestion() {
    let (env, client, _) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    // 1.0 at 6 decimals and 3.0 at 8 decimals → canonical 1.0 and 3.0.
    let prices = batch(
        &env,
        &[
            report(&env, &p, &a, 1_000_000, 6, 1, NOW),
            report(&env, &p, &b, 300_000_000, 8, 1, NOW),
        ],
    );
    assert_eq!(client.update_with_external_data(&prices), 20_000_000);
    assert_eq!(client.get_normalized_price(&p, &7), 20_000_000);
}

#[test]
fn external_report_must_match_configured_scale() {
    let (env, client, admin) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    client.set_feed_decimals(&admin, &p, &6);

    let wrong = batch(&env, &[report(&env, &p, &a, 2_000_000, 7, 1, NOW)]);
    assert_eq!(
        client.try_update_with_external_data(&wrong),
        Err(Ok(OracleError::UnsupportedPrecision))
    );
    assert_eq!(client.try_get_price(&p), Err(Ok(OracleError::PriceNotFound)));

    // Matching scale: the canonical consensus is returned and the stored
    // price is kept in the feed's native 6-decimal scale.
    let right = batch(&env, &[report(&env, &p, &a, 2_000_000, 6, 1, NOW)]);
    assert_eq!(client.update_with_external_data(&right), 20_000_000);
    assert_eq!(client.get_normalized_price(&p, &6), 2_000_000);
    assert_eq!(client.get_normalized_price(&p, &7), 20_000_000);
}

#[test]
fn external_report_with_unsupported_precision_is_rejected() {
    let (env, client, _) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    for decimals in [MAX_FEED_DECIMALS + 1, u32::MAX] {
        let prices = batch(&env, &[report(&env, &p, &a, 1, decimals, 1, NOW)]);
        assert_eq!(
            client.try_update_with_external_data(&prices),
            Err(Ok(OracleError::UnsupportedPrecision))
        );
    }
}

#[test]
fn external_report_overflow_is_rejected() {
    let (env, client, _) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    // 0 → 7 decimals multiplies by 10^7, which overflows for i128::MAX.
    let prices = batch(&env, &[report(&env, &p, &a, i128::MAX, 0, 1, NOW)]);
    assert_eq!(
        client.try_update_with_external_data(&prices),
        Err(Ok(OracleError::ConversionOverflow))
    );
}

#[test]
fn checked_normalization_overflow_boundaries() {
    let max_for_one_digit = i128::MAX / 10;
    assert_eq!(
        normalize_price_checked(max_for_one_digit, 6, 7),
        Ok(max_for_one_digit * 10)
    );
    assert_eq!(
        normalize_price_checked(max_for_one_digit + 1, 6, 7),
        Err(OracleError::ConversionOverflow)
    );

    let max_for_full_range = i128::MAX / 1_000_000_000_000_000_000;
    assert_eq!(
        normalize_price_checked(max_for_full_range, 0, 18),
        Ok(max_for_full_range * 1_000_000_000_000_000_000)
    );
    assert_eq!(
        normalize_price_checked(max_for_full_range + 1, 0, 18),
        Err(OracleError::ConversionOverflow)
    );

    // Downscaling never overflows and truncates toward zero.
    assert_eq!(normalize_price_checked(i128::MAX, 18, 7), Ok(i128::MAX / 100_000_000_000));
    assert_eq!(normalize_price_checked(19, 8, 7), Ok(1));
    assert_eq!(normalize_price_checked(i128::MAX, 7, 7), Ok(i128::MAX));

    // Unsupported precision on either side is rejected, not rescaled.
    assert_eq!(
        normalize_price_checked(1, MAX_FEED_DECIMALS + 1, 7),
        Err(OracleError::UnsupportedPrecision)
    );
    assert_eq!(
        normalize_price_checked(1, 7, MAX_FEED_DECIMALS + 1),
        Err(OracleError::UnsupportedPrecision)
    );
    // The raw helper refuses scale factors that cannot exist in i128.
    assert_eq!(rescale_price(1, 0, 39), None);
    assert_eq!(rescale_price(1, 0, u32::MAX), None);
}

// ── #1406: per-source heartbeat windows ──────────────────────────────────────

#[test]
fn heartbeat_boundary_just_before_at_and_after_expiry() {
    let (env, client, admin) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    client.add_price_source(&admin, &a, &1);
    client.set_feed_heartbeat(&admin, &p, &100);

    client.submit_pair_price(&a, &p, &1_000, &100);

    at(&env, NOW + 99); // just before expiry
    assert_eq!(client.get_price(&p), 1_000);

    at(&env, NOW + 100); // at expiry
    assert_eq!(client.try_get_price(&p), Err(Ok(OracleError::StalePrice)));

    at(&env, NOW + 101); // just after expiry
    assert_eq!(client.try_get_price(&p), Err(Ok(OracleError::StalePrice)));
}

#[test]
fn source_heartbeat_overrides_feed_default() {
    let (env, client, admin) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    client.add_price_source(&admin, &a, &1);
    client.add_price_source(&admin, &b, &1);
    client.set_feed_heartbeat(&admin, &p, &200);
    client.set_source_heartbeat(&admin, &p, &a, &100);
    assert_eq!(client.get_source_heartbeat(&p, &a), Some(100));
    assert_eq!(client.get_source_heartbeat(&p, &b), Some(200));

    client.submit_pair_price(&a, &p, &104, &100);
    client.submit_pair_price(&b, &p, &100, &100);

    at(&env, NOW + 50); // both live: median of [100, 104]
    assert_eq!(client.get_price(&p), 104);

    at(&env, NOW + 150); // `a` past its 100s override, `b` inside 200s
    assert_eq!(client.get_price(&p), 100);

    at(&env, NOW + 250); // both expired, though inside the 300s window
    assert_eq!(client.try_get_price(&p), Err(Ok(OracleError::StalePrice)));

    // Clearing the override falls back to the feed default.
    client.set_source_heartbeat(&admin, &p, &a, &0);
    assert_eq!(client.get_source_heartbeat(&p, &a), Some(200));
}

#[test]
fn expired_external_report_cannot_advance_round() {
    let (env, client, admin) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    client.set_source_heartbeat(&admin, &p, &a, &100);

    // Just before expiry: accepted and bound to round 1.
    let fresh = batch(&env, &[report(&env, &p, &a, 1_000, 7, 1, NOW - 99)]);
    assert_eq!(client.update_with_external_data(&fresh), 1_000);
    let accepted = client.get_price_round(&p);
    assert_eq!((accepted.round_id, accepted.price), (1, 1_000));

    // At and just after expiry: rejected, price and round unchanged.
    for (age, round) in [(100u64, 2u64), (101, 3)] {
        let stale = batch(&env, &[report(&env, &p, &a, 5_000, 7, round, NOW - age)]);
        assert_eq!(
            client.try_update_with_external_data(&stale),
            Err(Ok(OracleError::HeartbeatExpired))
        );
        assert_eq!(client.get_price_round(&p), accepted);
    }
}

// ── #1407: deterministic round identifiers ───────────────────────────────────

#[test]
fn round_identifier_is_stable_across_repeated_queries() {
    let (env, client, admin) = setup();
    let p = pair(&env, "USDC");
    let a = Address::generate(&env);
    client.add_price_source(&admin, &a, &1);

    assert_eq!(
        client.try_get_price_round(&p),
        Err(Ok(OracleError::PriceNotFound))
    );

    client.submit_pair_price(&a, &p, &1_000, &100);
    let first = client.get_price_round(&p);
    assert_eq!(
        first,
        PriceRound {
            round_id: 1,
            price: 1_000,
            timestamp: NOW,
        }
    );
    for _ in 0..3 {
        assert_eq!(client.get_price_round(&p), first);
    }

    // Time passing alone never changes the identifier.
    at(&env, NOW + 10);
    assert_eq!(client.get_price_round(&p), first);

    // A new accepted observation advances it monotonically.
    client.submit_pair_price(&a, &p, &1_010, &100);
    let second = client.get_price_round(&p);
    assert_eq!(
        second,
        PriceRound {
            round_id: 2,
            price: 1_010,
            timestamp: NOW + 10,
        }
    );
    assert!(second.round_id > first.round_id);
    assert_eq!(client.get_price_round(&p), second);
}

#[test]
fn round_identifiers_are_feed_local_across_ingestion_paths() {
    let (env, client, _) = setup();
    let usdc = pair(&env, "USDC");
    let btc = pair(&env, "BTC");

    client.set_price(&usdc, &1_000);
    client.set_price(&usdc, &1_001);
    client.set_price(&btc, &50_000);

    assert_eq!(client.get_price_round(&usdc).round_id, 2);
    assert_eq!(client.get_price_round(&btc).round_id, 1);
    assert_eq!(client.get_price_round(&usdc).price, 1_001);
}
