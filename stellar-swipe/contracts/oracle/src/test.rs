#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn xlm_asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    }
}

fn create_test_env() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let oracle1 = Address::generate(&env);
    let oracle2 = Address::generate(&env);
    let oracle3 = Address::generate(&env);

    (env, admin, oracle1, oracle2, oracle3)
}

#[test]
fn test_initialize() {
    let (env, admin, _, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));

    // Should panic on second init
    // client.initialize(&admin); // Uncomment to test panic
}

#[test]
fn test_register_oracle() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);

    let reputation = client.get_oracle_reputation(&oracle1);
    assert_eq!(reputation.reputation_score, 50);
    assert_eq!(reputation.weight, 1);
    assert_eq!(reputation.total_submissions, 0);
}

#[test]
fn test_submit_price() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);

    client.submit_price(&oracle1, &100_000_000);

    // Verify submission was recorded
    let _consensus = client.calculate_consensus();
    // Test passes if no panic occurs
}

#[test]
fn test_reputation_calculation_accurate_oracle() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);
    client.register_oracle(&admin, &oracle2);
    client.register_oracle(&admin, &oracle3);

    // Oracle1: accurate (100), Oracle2: moderate (105), Oracle3: poor (120)
    client.submit_price(&oracle1, &100_000_000);
    client.submit_price(&oracle2, &105_000_000);
    client.submit_price(&oracle3, &120_000_000);

    client.calculate_consensus();

    let rep1 = client.get_oracle_reputation(&oracle1);
    let rep2 = client.get_oracle_reputation(&oracle2);
    let rep3 = client.get_oracle_reputation(&oracle3);

    // All oracles should have submissions tracked
    assert_eq!(rep1.total_submissions, 1);
    assert_eq!(rep2.total_submissions, 1);
    assert_eq!(rep3.total_submissions, 1);

    // Oracle3 has highest deviation
    assert!(rep3.avg_deviation > rep1.avg_deviation);
}

#[test]
fn test_weight_adjustment() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);
    client.register_oracle(&admin, &oracle2);
    client.register_oracle(&admin, &oracle3);

    // Simulate multiple rounds with oracle1 being consistently accurate
    for _ in 0..10 {
        client.submit_price(&oracle1, &100_000_000);
        client.submit_price(&oracle2, &105_000_000);
        client.submit_price(&oracle3, &95_000_000);
        client.calculate_consensus();
    }

    let rep1 = client.get_oracle_reputation(&oracle1);

    // Oracle1 should have high weight due to accuracy
    assert!(rep1.weight >= 2);
    assert!(rep1.reputation_score >= 75);
}

#[test]
fn test_slash_for_major_deviation() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);
    client.register_oracle(&admin, &oracle2);
    client.register_oracle(&admin, &oracle3);

    // Oracle3 submits price with >20% deviation
    client.submit_price(&oracle1, &100_000_000);
    client.submit_price(&oracle2, &101_000_000);
    client.submit_price(&oracle3, &150_000_000); // 50% higher

    client.calculate_consensus();

    let rep3 = client.get_oracle_reputation(&oracle3);

    // Oracle3 should have reputation reduced due to slashing
    assert!(rep3.reputation_score < 50);
}

#[test]
fn test_oracle_removal_for_poor_performance() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);
    client.register_oracle(&admin, &oracle2);
    client.register_oracle(&admin, &oracle3);

    // Oracle3 consistently submits bad data until it gets weight 0
    for i in 0..50 {
        client.submit_price(&oracle1, &100_000_000);
        client.submit_price(&oracle2, &101_000_000);

        // Check if oracle3 still has weight before submitting
        let rep3 = client.get_oracle_reputation(&oracle3);
        if rep3.weight > 0 {
            client.submit_price(&oracle3, &200_000_000);
        }

        client.calculate_consensus();

        // Break early if oracle3 is already at weight 0
        if i > 10 && rep3.weight == 0 {
            break;
        }
    }

    let rep3 = client.get_oracle_reputation(&oracle3);

    // Oracle3 should eventually have weight 0 due to poor performance
    assert_eq!(rep3.weight, 0);
}

#[test]
fn test_reputation_recovery() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);
    client.register_oracle(&admin, &oracle2);
    client.register_oracle(&admin, &oracle3);

    // Oracle1 submits slightly inaccurate data initially (6% off - outside 5% threshold)
    for _ in 0..5 {
        client.submit_price(&oracle1, &106_000_000); // 6% off
        client.submit_price(&oracle2, &100_000_000);
        client.submit_price(&oracle3, &101_000_000);
        client.calculate_consensus();
    }

    let rep_before = client.get_oracle_reputation(&oracle1);

    // Oracle1 improves and becomes accurate
    for _ in 0..20 {
        client.submit_price(&oracle1, &100_000_000);
        client.submit_price(&oracle2, &100_500_000);
        client.submit_price(&oracle3, &101_000_000);
        client.calculate_consensus();
    }

    let rep_after = client.get_oracle_reputation(&oracle1);

    // Reputation should improve (more accurate submissions)
    assert!(rep_after.accurate_submissions > rep_before.accurate_submissions);
    assert_eq!(rep_after.total_submissions, 25); // 5 + 20
}

#[test]
fn test_weighted_median() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);
    client.register_oracle(&admin, &oracle2);
    client.register_oracle(&admin, &oracle3);

    // Build reputation for oracle1
    for _ in 0..10 {
        client.submit_price(&oracle1, &100_000_000);
        client.submit_price(&oracle2, &100_000_000);
        client.submit_price(&oracle3, &100_000_000);
        client.calculate_consensus();
    }

    let rep1 = client.get_oracle_reputation(&oracle1);

    // Oracle1 should have built up good reputation
    assert!(rep1.weight >= 1);
    assert_eq!(rep1.total_submissions, 10);
}

#[test]
fn test_minimum_oracles_maintained() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);
    client.register_oracle(&admin, &oracle2);
    client.register_oracle(&admin, &oracle3);

    let oracles_before = client.get_oracles();
    assert_eq!(oracles_before.len(), 3);

    // All oracles submit terrible data
    for i in 0..50 {
        let rep1 = client.get_oracle_reputation(&oracle1);
        let rep2 = client.get_oracle_reputation(&oracle2);
        let rep3 = client.get_oracle_reputation(&oracle3);

        // Only submit if oracle still has weight
        if rep1.weight > 0 {
            client.submit_price(&oracle1, &200_000_000);
        }
        if rep2.weight > 0 {
            client.submit_price(&oracle2, &300_000_000);
        }
        if rep3.weight > 0 {
            client.submit_price(&oracle3, &400_000_000);
        }

        // Need at least one submission to calculate consensus
        if rep1.weight == 0 && rep2.weight == 0 && rep3.weight == 0 {
            break;
        }

        client.calculate_consensus();
    }

    let oracles = client.get_oracles();

    // Should maintain at least 2 oracles in the registry even if all perform poorly
    assert!(oracles.len() >= 2);
}

#[test]
fn test_invalid_price_rejected() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);

    let result = client.try_submit_price(&oracle1, &0);
    assert_eq!(result, Err(Ok(OracleError::InvalidPrice)));

    let result = client.try_submit_price(&oracle1, &-100);
    assert_eq!(result, Err(Ok(OracleError::InvalidPrice)));
}

#[test]
fn test_unregistered_oracle_cannot_submit() {
    let (env, admin, _, _, _) = create_test_env();
    let unregistered = Address::generate(&env);
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));

    let result = client.try_submit_price(&unregistered, &100_000_000);
    assert_eq!(result, Err(Ok(OracleError::OracleNotFound)));
}

// ── Issue #602: minimum independent source count ─────────────────────────────

fn usdc_xlm_pair(env: &Env) -> stellar_swipe_common::AssetPair {
    use stellar_swipe_common::Asset;
    stellar_swipe_common::AssetPair {
        base: Asset {
            code: soroban_sdk::String::from_str(env, "USDC"),
            issuer: None,
        },
        quote: Asset {
            code: soroban_sdk::String::from_str(env, "XLM"),
            issuer: None,
        },
    }
}

#[test]
fn test_min_source_count_default_zero_allows_single_source() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.add_price_source(&admin, &oracle1, &1u32);

    let pair = usdc_xlm_pair(&env);
    client.submit_pair_price(&oracle1, &pair, &1_000_000, &100u32);

    // With default min_source_count=0, one source is enough.
    let result = client.try_get_price_with_confidence(&pair);
    assert!(result.is_ok());
}

#[test]
fn test_below_min_sources_returns_insufficient_sources() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.add_price_source(&admin, &oracle1, &1u32);

    // Require at least 2 sources.
    client.set_min_source_count(&admin, &2u32);
    assert_eq!(client.get_min_source_count(), 2u32);

    let pair = usdc_xlm_pair(&env);
    client.submit_pair_price(&oracle1, &pair, &1_000_000, &100u32);

    // Only 1 fresh source — should fail with InsufficientSources.
    let err = client.try_get_price_with_confidence(&pair);
    assert!(err.is_err());
}

#[test]
fn test_at_min_sources_accepted() {
    let (env, admin, oracle1, oracle2, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.add_price_source(&admin, &oracle1, &1u32);
    client.add_price_source(&admin, &oracle2, &1u32);

    client.set_min_source_count(&admin, &2u32);

    let pair = usdc_xlm_pair(&env);
    client.submit_pair_price(&oracle1, &pair, &1_000_000, &100u32);
    client.submit_pair_price(&oracle2, &pair, &1_000_000, &100u32);

    // Exactly 2 sources — must be accepted.
    let result = client.try_get_price_with_confidence(&pair);
    assert!(result.is_ok());
}

#[test]
fn test_above_min_sources_accepted() {
    let (env, admin, oracle1, oracle2, oracle3) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));
    client.add_price_source(&admin, &oracle1, &1u32);
    client.add_price_source(&admin, &oracle2, &1u32);
    client.add_price_source(&admin, &oracle3, &1u32);

    client.set_min_source_count(&admin, &2u32);

    let pair = usdc_xlm_pair(&env);
    client.submit_pair_price(&oracle1, &pair, &1_000_000, &100u32);
    client.submit_pair_price(&oracle2, &pair, &1_000_000, &100u32);
    client.submit_pair_price(&oracle3, &pair, &1_000_000, &100u32);

    // 3 sources >= min 2 — accepted.
    let result = client.try_get_price_with_confidence(&pair);
    assert!(result.is_ok());
}

#[test]
fn test_min_source_count_admin_only() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));

    // Non-admin should not be able to set min source count.
    let result = client.try_set_min_source_count(&oracle1, &3u32);
    assert!(result.is_err());
}

// ── Oracle price normalisation tests ─────────────────────────────────────────

fn make_pair(env: &Env, base: &str, quote: &str) -> stellar_swipe_common::AssetPair {
    stellar_swipe_common::AssetPair {
        base: stellar_swipe_common::Asset {
            code: String::from_str(env, base),
            issuer: None,
        },
        quote: stellar_swipe_common::Asset {
            code: String::from_str(env, quote),
            issuer: None,
        },
    }
}

#[test]
fn test_set_and_get_feed_decimals() {
    let (env, admin, _, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));

    let pair = make_pair(&env, "XLM", "USDC");
    assert_eq!(client.get_feed_decimals(&pair), None);

    client.set_feed_decimals(&admin, &pair, &7u32);
    assert_eq!(client.get_feed_decimals(&pair), Some(7u32));
}

#[test]
fn test_set_feed_decimals_non_admin_fails() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));

    let pair = make_pair(&env, "XLM", "USDC");
    let result = client.try_set_feed_decimals(&oracle1, &pair, &7u32);
    assert!(result.is_err());
}

#[test]
fn test_get_normalized_price_same_decimals() {
    let (env, admin, _, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));

    let pair = make_pair(&env, "XLM", "USDC");
    // Store price 1_000_000 (6 decimals → 1.0 USDC).
    client.set_price(&pair, &1_000_000i128);
    client.set_feed_decimals(&admin, &pair, &6u32);

    // Requesting target_decimals == native_decimals → no rescaling.
    let price = client.get_normalized_price(&pair, &6u32);
    assert_eq!(price, 1_000_000i128);
}

#[test]
fn test_get_normalized_price_scale_up() {
    let (env, admin, _, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));

    // Asset A has 6 native decimals; caller wants 8 target decimals.
    let pair_a = make_pair(&env, "USDC", "USD");
    client.set_price(&pair_a, &1_000_000i128); // 1.000000 USDC
    client.set_feed_decimals(&admin, &pair_a, &6u32);

    let norm = client.get_normalized_price(&pair_a, &8u32);
    assert_eq!(norm, 100_000_000i128); // 1.000000 → 1.00000000
}

#[test]
fn test_get_normalized_price_scale_down() {
    let (env, admin, _, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));

    // Asset B has 8 native decimals; caller wants 6 target decimals.
    let pair_b = make_pair(&env, "BTC", "USD");
    client.set_price(&pair_b, &6_000_000_000_000i128); // 60_000.00000000 USD (8 dec)
    client.set_feed_decimals(&admin, &pair_b, &8u32);

    let norm = client.get_normalized_price(&pair_b, &6u32);
    assert_eq!(norm, 60_000_000_000i128); // 60_000.000000 (6 dec)
}

#[test]
fn test_get_normalized_price_differing_native_decimals_to_common_target() {
    let (env, admin, _, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));

    let pair_xlm = make_pair(&env, "XLM", "USD");
    let pair_usdc = make_pair(&env, "USDC", "USD");

    // XLM stored with 7 decimals (Stellar native), USDC with 6 decimals.
    client.set_price(&pair_xlm, &11_000_000_i128); // 1.1000000 USD (7 dec)
    client.set_feed_decimals(&admin, &pair_xlm, &7u32);

    client.set_price(&pair_usdc, &1_000_000i128); // 1.000000 USD (6 dec)
    client.set_feed_decimals(&admin, &pair_usdc, &6u32);

    // Normalise both to 8 decimals for consistent comparison.
    let xlm_norm = client.get_normalized_price(&pair_xlm, &8u32);
    let usdc_norm = client.get_normalized_price(&pair_usdc, &8u32);

    assert_eq!(xlm_norm, 110_000_000i128); // 1.10000000
    assert_eq!(usdc_norm, 100_000_000i128); // 1.00000000
                                            // XLM is now priced above USDC, as expected.
    assert!(xlm_norm > usdc_norm);
}

#[test]
fn test_get_normalized_price_no_decimals_configured_returns_error() {
    let (env, admin, _, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));

    let pair = make_pair(&env, "ETH", "USD");
    client.set_price(&pair, &2_000_000_000i128);
    // Deliberately omit set_feed_decimals.

    let result = client.try_get_normalized_price(&pair, &6u32);
    assert!(result.is_err());
}

#[test]
fn error_messages_are_non_empty_and_distinct() {
    let samples = [
        OracleError::PriceNotFound,
        OracleError::Unauthorized,
        OracleError::StalePrice,
        OracleError::InsufficientSources,
        OracleError::PriceDeviationBreakerTripped,
    ];
    for err in samples.iter() {
        assert!(!err.message().is_empty());
    }
    for i in 0..samples.len() {
        for j in (i + 1)..samples.len() {
            assert_ne!(
                samples[i].message(),
                samples[j].message(),
                "expected distinct messages for {:?} and {:?}",
                samples[i],
                samples[j]
            );
        }
    }
}

// ── Instruction-budget regression snapshots (Issue #budget) ───────────────────

use stellar_swipe_common::budget_regression::measure_and_emit;

#[test]
fn set_price_budget_regression() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);

    env.budget().reset_tracker();
    client.set_price(
        AssetPair {
            base: xlm_asset(&env),
            quote: Asset {
                code: String::from_str(&env, "USDC"),
                issuer: Some(Address::generate(&env)),
            },
        },
        100_000_000,
    );
    let instructions = env.budget().cpu_instruction_cost();
    measure_and_emit("oracle.set_price", 3_000_000, instructions);
}

#[test]
fn get_price_budget_regression() {
    let (env, admin, oracle1, _, _) = create_test_env();
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);
    client.initialize(&admin, &xlm_asset(&env));
    client.register_oracle(&admin, &oracle1);

    let pair = AssetPair {
        base: xlm_asset(&env),
        quote: Asset {
            code: String::from_str(&env, "USDC"),
            issuer: Some(Address::generate(&env)),
        },
    };
    client.set_price(pair.clone(), 100_000_000);
    client.submit_price(&oracle1, &100_000_000);

    env.budget().reset_tracker();
    let _ = client.get_price(pair);
    let instructions = env.budget().cpu_instruction_cost();
    measure_and_emit("oracle.get_price", 2_000_000, instructions);
}
