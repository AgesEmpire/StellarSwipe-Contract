use soroban_sdk::{Address, Env, Map};
use stellar_swipe_common::{BASIS_POINTS_DENOMINATOR_I128, SECONDS_PER_WEEK};

use crate::types::{OracleReputation, StorageKey};

const ACCURACY_THRESHOLD_TIGHT: i128 = 100; // 1% in basis points
const ACCURACY_THRESHOLD_MODERATE: i128 = 500; // 5% in basis points
const MAJOR_DEVIATION_THRESHOLD: i128 = 2000; // 20% in basis points

pub fn get_oracle_stats(env: &Env, oracle: &Address) -> OracleReputation {
    let stats_map: Map<Address, OracleReputation> = env
        .storage()
        .persistent()
        .get(&StorageKey::OracleStats)
        .unwrap_or(Map::new(env));

    stats_map.get(oracle.clone()).unwrap_or(OracleReputation {
        total_submissions: 0,
        accurate_submissions: 0,
        avg_deviation: 0,
        reputation_score: 50,
        weight: 1,
        last_slash: 0,
    })
}

pub fn save_oracle_stats(env: &Env, oracle: &Address, stats: &OracleReputation) {
    let mut stats_map: Map<Address, OracleReputation> = env
        .storage()
        .persistent()
        .get(&StorageKey::OracleStats)
        .unwrap_or(Map::new(env));

    stats_map.set(oracle.clone(), stats.clone());
    env.storage()
        .persistent()
        .set(&StorageKey::OracleStats, &stats_map);
}

pub fn calculate_reputation(env: &Env, oracle: &Address) -> u32 {
    let stats = get_oracle_stats(env, oracle);

    if stats.total_submissions == 0 {
        return 50; // Default for new oracles
    }

    // 60% based on accuracy rate
    let accuracy_score = (stats.accurate_submissions * 60) / stats.total_submissions;

    // 30% based on deviation (lower is better)
    let deviation_penalty = (stats.avg_deviation / 1000).min(30);
    let deviation_score = 30_i128.saturating_sub(deviation_penalty);

    // 10% based on consistency (no recent slashes)
    let consistency_score = if env.ledger().timestamp() - stats.last_slash > SECONDS_PER_WEEK {
        10
    } else {
        0
    };

    let total = accuracy_score as i128 + deviation_score + consistency_score as i128;
    total.min(100).max(0) as u32
}

pub fn adjust_oracle_weight(env: &Env, oracle: &Address) -> u32 {
    let reputation = calculate_reputation(env, oracle);

    let new_weight = match reputation {
        90..=100 => 10,
        75..=89 => 5,
        60..=74 => 2,
        50..=59 => 1,
        _ => 0,
    };

    let mut stats = get_oracle_stats(env, oracle);
    stats.weight = new_weight;
    stats.reputation_score = reputation;
    save_oracle_stats(env, oracle, &stats);

    new_weight
}

pub fn track_oracle_accuracy(
    env: &Env,
    oracle: &Address,
    submitted_price: i128,
    consensus_price: i128,
) {
    let mut stats = get_oracle_stats(env, oracle);

    stats.total_submissions += 1;

    // Calculate deviation in basis points
    let deviation = ((submitted_price - consensus_price).abs() * BASIS_POINTS_DENOMINATOR_I128)
        / consensus_price;

    // Update average deviation
    let total_dev = stats.avg_deviation * (stats.total_submissions - 1) as i128;
    stats.avg_deviation = (total_dev + deviation) / stats.total_submissions as i128;

    // Check accuracy
    if deviation <= ACCURACY_THRESHOLD_TIGHT {
        stats.accurate_submissions += 1;
    } else if deviation <= ACCURACY_THRESHOLD_MODERATE {
        stats.accurate_submissions += 1; // Moderately accurate still counts
    }

    save_oracle_stats(env, oracle, &stats);
}

pub fn slash_oracle(env: &Env, oracle: &Address, reason: SlashReason) {
    let mut stats = get_oracle_stats(env, oracle);

    let penalty = match reason {
        SlashReason::MajorDeviation => 20,
        SlashReason::SignatureFailure => 30,
    };

    stats.reputation_score = stats.reputation_score.saturating_sub(penalty);
    stats.last_slash = env.ledger().timestamp();

    save_oracle_stats(env, oracle, &stats);
}

pub fn should_remove_oracle(env: &Env, oracle: &Address) -> bool {
    let stats = get_oracle_stats(env, oracle);

    // Remove if consistently inaccurate
    if stats.total_submissions >= 100 {
        let accuracy_rate = (stats.accurate_submissions * 100) / stats.total_submissions;
        if accuracy_rate < 50 {
            return true;
        }
    }

    // Remove if reputation too low
    let reputation = calculate_reputation(env, oracle);
    reputation < 50
}

pub enum SlashReason {
    MajorDeviation,
    SignatureFailure,
}
