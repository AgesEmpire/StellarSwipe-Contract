//! Trust score calculation for signal providers
//!
//! Trust score combines multiple factors to provide a holistic reliability metric:
//! - 40%: Success rate (successful_signals / total_signals)
//! - 20%: Consistency (inverse of ROI variance across signals)
//! - 15%: Stake amount (normalized vs median stake)
//! - 15%: Follower count (normalized vs median followers)
//! - 10%: Tenure (days since first signal)
//!
//! Score ranges from 0-100, with tiers:
//! - 80-100: "Highly Trusted" (green badge)
//! - 60-79: "Trusted" (blue badge)
//! - 40-59: "Emerging" (yellow badge)
//! - 0-39: "New/Unproven" (gray badge)

use soroban_sdk::{contracttype, Address, Env, Map, Vec};
use crate::types::ProviderPerformance;
use crate::stake::StakeInfo;
use crate::social;

const TRUST_SCORE_SCALE: u32 = 100; // 0-100 scale
const MIN_SIGNALS_FOR_TRUST_SCORE: u32 = 5; // Minimum signals to calculate trust score

// Trust score component weights (in basis points, total = 10000)
const SUCCESS_RATE_WEIGHT: u32 = 4000; // 40%
const CONSISTENCY_WEIGHT: u32 = 2000;  // 20%
const STAKE_WEIGHT: u32 = 1500;        // 15%
const FOLLOWER_WEIGHT: u32 = 1500;     // 15%
const TENURE_WEIGHT: u32 = 1000;       // 10%

// Maximum tenure days for full score (365 days = 100%)
const MAX_TENURE_DAYS: u64 = 365;
// Seconds per day
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustScoreTier {
    HighlyTrusted, // 80-100
    Trusted,       // 60-79
    Emerging,      // 40-59
    NewUnproven,   // 0-39
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TrustScoreComponents {
    pub success_rate: u32,        // 0-10000 (basis points)
    pub consistency: u32,         // 0-10000 (basis points)
    pub stake_normalized: u32,    // 0-10000 (basis points)
    pub followers_normalized: u32, // 0-10000 (basis points)
    pub tenure_normalized: u32,   // 0-10000 (basis points)
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TrustScoreDetails {
    pub score: u32,               // 0-100
    pub tier: TrustScoreTier,
    pub components: TrustScoreComponents,
    pub has_sufficient_history: bool, // true if >= MIN_SIGNALS_FOR_TRUST_SCORE
    pub last_updated: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum ReputationDataKey {
    /// provider -> TrustScoreDetails
    TrustScore(Address),
    /// provider -> u64 (timestamp of first signal)
    FirstSignalTime(Address),
    /// Global median stake amount for normalization
    MedianStake,
    /// Global median follower count for normalization
    MedianFollowers,
}

/// Calculate trust score for a provider
///
/// # Arguments
/// * `env` - Soroban environment
/// * `provider` - Provider address
/// * `performance` - Provider performance data
/// * `stake_info` - Provider stake information
///
/// # Returns
/// TrustScoreDetails with score, tier, and component breakdown
pub fn calculate_trust_score(
    env: &Env,
    provider: &Address,
    performance: &ProviderPerformance,
    stake_info: &Option<StakeInfo>,
) -> TrustScoreDetails {
    let now = env.ledger().timestamp();

    // Check if provider has sufficient history
    let has_sufficient_history = performance.total_signals >= MIN_SIGNALS_FOR_TRUST_SCORE;

    // If insufficient history, return zero score
    if !has_sufficient_history {
        return TrustScoreDetails {
            score: 0,
            tier: TrustScoreTier::NewUnproven,
            components: TrustScoreComponents {
                success_rate: 0,
                consistency: 0,
                stake_normalized: 0,
                followers_normalized: 0,
                tenure_normalized: 0,
            },
            has_sufficient_history: false,
            last_updated: now,
        };
    }

    // Calculate individual components
    let success_rate_component = calculate_success_rate_component(performance);
    let consistency_component = calculate_consistency_component(performance);
    let stake_component = calculate_stake_component(env, stake_info);
    let follower_component = calculate_follower_component(env, provider);
    let tenure_component = calculate_tenure_component(env, provider, now);

    // Combine components using weights
    let score = calculate_weighted_score(
        success_rate_component,
        consistency_component,
        stake_component,
        follower_component,
        tenure_component,
    );

    let tier = get_trust_score_tier(score);

    TrustScoreDetails {
        score,
        tier,
        components: TrustScoreComponents {
            success_rate: success_rate_component,
            consistency: consistency_component,
            stake_normalized: stake_component,
            followers_normalized: follower_component,
            tenure_normalized: tenure_component,
        },
        has_sufficient_history: true,
        last_updated: now,
    }
}

/// Calculate success rate component (40% weight)
/// Returns 0-10000 basis points
fn calculate_success_rate_component(performance: &ProviderPerformance) -> u32 {
    if performance.total_signals == 0 {
        return 0;
    }

    // success_rate is already in basis points (0-10000)
    performance.success_rate
}

/// Calculate consistency component (20% weight)
/// Measures inverse of ROI variance - lower variance = higher consistency
/// Returns 0-10000 basis points
fn calculate_consistency_component(performance: &ProviderPerformance) -> u32 {
    if performance.total_signals < 2 {
        return 5000; // Neutral score for providers with limited history
    }

    // Use success rate plus average return as a proxy for consistency.
    // In a full implementation, this should be based on variance of individual ROIs.
    let success_rate = performance.success_rate.min(10000);

    // Normalize avg_return (-10000..10000) to 0..10000
    let avg_return_clamped = performance.avg_return.max(-10000).min(10000);
    let avg_return_score = ((avg_return_clamped + 10000) * 10000 / 20000) as u32;

    // Blend success and avg return (60/40) for an approximation of consistency
    let consistency = (success_rate as u64 * 60 + avg_return_score as u64 * 40) / 100;
    consistency.min(10000) as u32
}

/// Calculate stake component (15% weight)
/// Normalized against median stake amount
/// Returns 0-10000 basis points
pub(crate) fn calculate_stake_component(env: &Env, stake_info: &Option<StakeInfo>) -> u32 {
    let stake_amount = match stake_info {
        Some(info) => info.amount,
        None => 0,
    };

    if stake_amount <= 0 {
        return 0;
    }

    let median_stake = get_median_stake(env);

    if median_stake <= 0 {
        return 5000; // Neutral if no median available
    }

    // Normalize: stake_amount / median_stake, capped at 2x median
    let ratio = if stake_amount > median_stake * 2 {
        2_0000 // 200% of median = 100% score
    } else {
        (stake_amount as u64 * 10000 / median_stake as u64) as u32
    };

    ratio.min(10000)
}

/// Calculate follower component (15% weight)
/// Normalized against median follower count
/// Returns 0-10000 basis points
fn calculate_follower_component(env: &Env, provider: &Address) -> u32 {
    let follower_count = social::get_follower_count(env, provider) as u64;

    let median_followers = get_median_followers(env);

    if median_followers <= 0 {
        return 5000; // Neutral if no median available
    }

    // Normalize: follower_count / median_followers, capped at 3x median
    let ratio = if follower_count > median_followers * 3 {
        3_0000 // 300% of median = 100% score
    } else {
        (follower_count * 10000 / median_followers) as u32
    };

    ratio.min(10000)
}

/// Calculate tenure component (10% weight)
/// Days since first signal, normalized to 365 days max
/// Returns 0-10000 basis points
pub(crate) fn calculate_tenure_component(env: &Env, provider: &Address, now: u64) -> u32 {
    let first_signal_time = get_first_signal_time(env, provider);

    if first_signal_time == 0 {
        return 0;
    }

    let days_since_first = now.saturating_sub(first_signal_time) / SECONDS_PER_DAY;

    if days_since_first >= MAX_TENURE_DAYS {
        10000 // Max score for established providers
    } else {
        ((days_since_first as u64 * 10000) / MAX_TENURE_DAYS) as u32
    }
}

/// Calculate weighted trust score from components
pub(crate) fn calculate_weighted_score(
    success_rate: u32,
    consistency: u32,
    stake: u32,
    followers: u32,
    tenure: u32,
) -> u32 {
    let num = success_rate as u64 * SUCCESS_RATE_WEIGHT as u64
        + consistency as u64 * CONSISTENCY_WEIGHT as u64
        + stake as u64 * STAKE_WEIGHT as u64
        + followers as u64 * FOLLOWER_WEIGHT as u64
        + tenure as u64 * TENURE_WEIGHT as u64;
    let score = (num / 1_000_000).min(TRUST_SCORE_SCALE as u64) as u32;
    score
}

/// Get trust score tier from score
pub(crate) fn get_trust_score_tier(score: u32) -> TrustScoreTier {
    match score {
        80..=100 => TrustScoreTier::HighlyTrusted,
        60..=79 => TrustScoreTier::Trusted,
        40..=59 => TrustScoreTier::Emerging,
        _ => TrustScoreTier::NewUnproven,
    }
}

/// Get stored trust score for provider
pub fn get_trust_score(env: &Env, provider: &Address) -> Option<TrustScoreDetails> {
    env.storage()
        .persistent()
        .get(&ReputationDataKey::TrustScore(provider.clone()))
}

/// Store trust score for provider
pub fn store_trust_score(env: &Env, provider: &Address, score: &TrustScoreDetails) {
    env.storage()
        .persistent()
        .set(&ReputationDataKey::TrustScore(provider.clone()), score);
}

/// Update trust score for provider (called when performance changes)
pub fn update_trust_score(
    env: &Env,
    provider: &Address,
    performance: &ProviderPerformance,
    stake_info: &Option<StakeInfo>,
) {
    let score_details = calculate_trust_score(env, provider, performance, stake_info);
    store_trust_score(env, provider, &score_details);
}

/// Record first signal time for tenure calculation
pub fn record_first_signal(env: &Env, provider: &Address) {
    let key = ReputationDataKey::FirstSignalTime(provider.clone());
    let existing_time: Option<u64> = env.storage().persistent().get(&key);

    if existing_time.is_none() {
        let now = env.ledger().timestamp();
        env.storage().persistent().set(&key, &now);
    }
}

/// Get first signal time for provider
pub(crate) fn get_first_signal_time(env: &Env, provider: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&ReputationDataKey::FirstSignalTime(provider.clone()))
        .unwrap_or(0)
}

/// Get provider performance (helper function)
fn get_provider_performance(env: &Env, provider: &Address) -> ProviderPerformance {
    // This would typically come from analytics or performance module
    // For now, return default - in real implementation, this would query the actual data
    env.storage()
        .instance()
        .get(&crate::StorageKey::ProviderStats)
        .and_then(|stats: Map<Address, ProviderPerformance>| stats.get(provider.clone()))
        .unwrap_or_default()
}

/// Get median stake amount across all providers
fn get_median_stake(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&ReputationDataKey::MedianStake)
        .unwrap_or(100_000_000) // Default 100 XLM
}

/// Get median follower count across all providers
fn get_median_followers(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&ReputationDataKey::MedianFollowers)
        .unwrap_or(50) // Default 50 followers
}

/// Update global median values (called periodically by admin)
pub fn update_median_values(env: &Env, median_stake: i128, median_followers: u64) {
    env.storage()
        .persistent()
        .set(&ReputationDataKey::MedianStake, &median_stake);
    env.storage()
        .persistent()
        .set(&ReputationDataKey::MedianFollowers, &median_followers);
}

/// Get all trust scores for leaderboard
pub fn get_all_trust_scores(env: &Env) -> Vec<(Address, TrustScoreDetails)> {
    let provider_stats: Map<Address, ProviderPerformance> = env
        .storage()
        .instance()
        .get(&crate::StorageKey::ProviderStats)
        .unwrap_or(Map::new(env));

    let mut results = Vec::new(env);
    for provider in provider_stats.keys() {
        if let Some(performance) = provider_stats.get(provider.clone()) {
            let stake_info = crate::stake::get_stake_info(env, &provider);
            let trust_score = calculate_trust_score(env, &provider, &performance, &stake_info);
            results.push_back((provider.clone(), trust_score));
        }
    }

    results
}


/// Points for weighted reputation update (Issue #170): profit 100, neutral 50, loss 0.
pub fn outcome_points(outcome: &crate::types::SignalOutcome) -> u32 {
    match outcome {
        crate::types::SignalOutcome::Profit => 100,
        crate::types::SignalOutcome::Neutral => 50,
        crate::types::SignalOutcome::Loss => 0,
    }
}

/// Integer form of `new = old * 0.9 + outcome * 0.1` on a 0–100 scale.
pub fn next_reputation_score(old_score: u32, outcome: &crate::types::SignalOutcome) -> u32 {
    let pts = outcome_points(outcome);
    (((old_score as u64) * 9 + (pts as u64)) / 10) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[test]
    fn test_trust_score_tiers() {
        assert_eq!(get_trust_score_tier(85), TrustScoreTier::HighlyTrusted);
        assert_eq!(get_trust_score_tier(75), TrustScoreTier::Trusted);
        assert_eq!(get_trust_score_tier(50), TrustScoreTier::Emerging);
        assert_eq!(get_trust_score_tier(25), TrustScoreTier::NewUnproven);
    }

    #[test]
    fn test_calculate_weighted_score() {
        // Test with all components at 100% (10000 basis points)
        let score = calculate_weighted_score(10000, 10000, 10000, 10000, 10000);
        assert_eq!(score, 100);

        // Test with all components at 50% (5000 basis points)
        let score = calculate_weighted_score(5000, 5000, 5000, 5000, 5000);
        assert_eq!(score, 50);
    }

    #[test]
    fn test_insufficient_history() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let performance = ProviderPerformance {
            total_signals: 3, // Less than MIN_SIGNALS_FOR_TRUST_SCORE
            successful_signals: 2,
            failed_signals: 1,
            total_copies: 10,
            success_rate: 6667, // 66.67%
            avg_return: 500,
            total_volume: 1000000,
        };

        let score_details = calculate_trust_score(&env, &provider, &performance, &None);
        assert_eq!(score_details.score, 0);
        assert_eq!(score_details.tier, TrustScoreTier::NewUnproven);
        assert!(!score_details.has_sufficient_history);
    }

    #[test]
    fn test_success_rate_component() {
        let performance = ProviderPerformance {
            total_signals: 10,
            success_rate: 7500, // 75%
            ..Default::default()
        };

        let component = calculate_success_rate_component(&performance);
        assert_eq!(component, 7500);
    }

    #[test]
    fn test_stake_component() {
        let env = Env::default();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || {
            // Set median stake to 100 XLM
            update_median_values(&env, 100_000_000, 50);

            let stake_info = Some(StakeInfo {
                amount: 200_000_000,
                last_signal_time: 0,
                locked_until: 0,
            });
            let component = calculate_stake_component(&env, &stake_info);
            assert_eq!(component, 10000);

            let stake_info = Some(StakeInfo {
                amount: 50_000_000,
                last_signal_time: 0,
                locked_until: 0,
            });
            let component = calculate_stake_component(&env, &stake_info);
            assert_eq!(component, 5000);
        });
    }

    #[test]
    fn test_tenure_component() {
        let env = Env::default();
        env.ledger().set_timestamp(500_000_000);
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || {
            let provider = Address::generate(&env);
            let now = env.ledger().timestamp();
            let first_signal_time = now - (100 * SECONDS_PER_DAY);
            env.storage()
                .persistent()
                .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &first_signal_time);
            let component = calculate_tenure_component(&env, &provider, now);
            assert_eq!(component, 2739);
        });
    }

    #[test]
    fn test_consistency_component() {
        // Insufficient history returns neutral 5000
        let performance = ProviderPerformance {
            total_signals: 1,
            ..Default::default()
        };
        assert_eq!(calculate_consistency_component(&performance), 5000);

        // Positive returns and strong success rate should increase consistency.
        let performance = ProviderPerformance {
            total_signals: 10,
            success_rate: 8000,
            avg_return: 5000,
            ..Default::default()
        };
        assert_eq!(calculate_consistency_component(&performance), 7800);

        // Negative returns reduce consistency.
        let performance = ProviderPerformance {
            total_signals: 10,
            success_rate: 5000,
            avg_return: -8000,
            ..Default::default()
        };
        assert_eq!(calculate_consistency_component(&performance), 3400);
    }

    #[test]
    fn test_get_all_trust_scores() {
        let env = Env::default();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || {
            let provider = Address::generate(&env);
            let mut provider_stats: Map<Address, ProviderPerformance> = Map::new(&env);
            provider_stats.set(
                provider.clone(),
                ProviderPerformance {
                    total_signals: 5,
                    successful_signals: 4,
                    failed_signals: 1,
                    success_rate: 8000,
                    avg_return: 5000,
                    ..Default::default()
                },
            );
            env.storage()
                .instance()
                .set(&crate::StorageKey::ProviderStats, &provider_stats);
            let list = get_all_trust_scores(&env);
            assert_eq!(list.len(), 1);
            assert_eq!(list.get(0).unwrap().0, provider);
            assert_eq!(list.get(0).unwrap().1.has_sufficient_history, true);
        });
    }
}