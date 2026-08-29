#![cfg(test)]

//! Unit tests for Issue #755: single-update price-deviation circuit breaker.

#[cfg(not(target_family = "wasm"))]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn xlm_usdc(env: &Env) -> AssetPair {
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

fn setup(env: &Env) -> (Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(env, &id);
    client.initialize(
        &admin,
        &Asset {
            code: String::from_str(env, "XLM"),
            issuer: None,
        },
    );
    (admin, id)
}

/// A normal incremental price update that stays within the threshold is accepted.
#[test]
fn test_normal_update_accepted() {
    let env = Env::default();
    let (admin, id) = setup(&env);
    let client = OracleContractClient::new(&env, &id);
    let pair = xlm_usdc(&env);

    // Set threshold: 20% max deviation.
    client.set_update_deviation_threshold(&admin, &pair, &2_000u32);

    // First price — always accepted (no prior price).
    client.set_price(&pair, &1_000_000i128);

    // 5% increase — within threshold.
    client.set_price(&pair, &1_050_000i128);

    assert!(!client.is_update_dev_breaker_tripped(&pair));
}

/// An update exceeding the threshold trips the breaker and subsequent
/// price-dependent calls are rejected.
#[test]
fn test_spike_trips_breaker_and_blocks_get_price() {
    let env = Env::default();
    let (admin, id) = setup(&env);
    let client = OracleContractClient::new(&env, &id);
    let pair = xlm_usdc(&env);

    client.set_update_deviation_threshold(&admin, &pair, &500u32); // 5% threshold

    // Establish baseline.
    client.set_price(&pair, &1_000_000i128);

    // 50% spike — trips the breaker. The update call must succeed (return Ok)
    // so the trip flag persists; the spiked price is NOT stored.
    let result = client.try_set_price(&pair, &1_500_000i128);
    assert!(
        result.is_ok(),
        "trip call must succeed so the flag persists"
    );

    // Breaker must now be tripped.
    assert!(client.is_update_dev_breaker_tripped(&pair));

    // Price reads are rejected while the breaker is tripped.
    assert!(client.try_get_price(&pair).is_err());
}

/// After an authorized reset the breaker clears and normal operations resume.
#[test]
fn test_authorized_reset_restores_normal_operation() {
    let env = Env::default();
    let (admin, id) = setup(&env);
    let client = OracleContractClient::new(&env, &id);
    let pair = xlm_usdc(&env);

    client.set_update_deviation_threshold(&admin, &pair, &500u32); // 5%

    client.set_price(&pair, &1_000_000i128);
    // Trip the breaker.
    let _ = client.try_set_price(&pair, &2_000_000i128);
    assert!(client.is_update_dev_breaker_tripped(&pair));

    // Admin resets it.
    client.reset_update_deviation_breaker(&admin, &pair);
    assert!(!client.is_update_dev_breaker_tripped(&pair));

    // Normal update should now succeed.
    client.set_price(&pair, &1_000_000i128);
}

/// When no threshold is configured (0), any price update is accepted.
#[test]
fn test_no_threshold_allows_any_update() {
    let env = Env::default();
    let (admin, id) = setup(&env);
    let client = OracleContractClient::new(&env, &id);
    let pair = xlm_usdc(&env);

    // No threshold set — default is 0 (disabled).
    client.set_price(&pair, &1_000_000i128);
    client.set_price(&pair, &9_999_999i128); // 900% move — allowed

    assert!(!client.is_update_dev_breaker_tripped(&pair));
}

/// Breaker event is emitted when it trips; reset event is emitted on reset.
///
/// Note: the test env clears the event buffer at the start of every top-level
/// contract invocation, so events must be inspected immediately after the call
/// that emits them (before any further invocation).
#[test]
fn test_breaker_events_emitted() {
    use soroban_sdk::{testutils::Events, Symbol, TryFromVal};

    let env = Env::default();
    let (admin, id) = setup(&env);
    let client = OracleContractClient::new(&env, &id);
    let pair = xlm_usdc(&env);

    client.set_update_deviation_threshold(&admin, &pair, &500u32);
    client.set_price(&pair, &1_000_000i128);

    let event_names = |env: &Env| -> std::vec::Vec<Symbol> {
        env.events()
            .all()
            .iter()
            .filter_map(|(_, topics, _)| topics.get(1))
            .filter_map(|t| Symbol::try_from_val(env, &t).ok())
            .collect()
    };

    // Trip the breaker — dev_trip must be the event of this invocation.
    let _ = client.try_set_price(&pair, &2_000_000i128);
    let trip_names = event_names(&env);
    assert!(
        trip_names
            .iter()
            .any(|n| n == &Symbol::new(&env, "dev_trip")),
        "dev_trip event not emitted, got {:?}",
        trip_names
    );

    // Reset the breaker — dev_reset must be the event of this invocation.
    client.reset_update_deviation_breaker(&admin, &pair);
    let reset_names = event_names(&env);
    assert!(
        reset_names
            .iter()
            .any(|n| n == &Symbol::new(&env, "dev_reset")),
        "dev_reset event not emitted, got {:?}",
        reset_names
    );
}
