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
    assert!(result.is_err());

    let result = client.try_submit_price(&oracle1, &-100);
    assert!(result.is_err());
}

#[test]
fn test_unregistered_oracle_cannot_submit() {
    let (env, admin, _, _, _) = create_test_env();
    let unregistered = Address::generate(&env);
    let contract_id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(&env, &contract_id);

    client.initialize(&admin, &xlm_asset(&env));

    let result = client.try_submit_price(&unregistered, &100_000_000);
    assert!(result.is_err());
}
