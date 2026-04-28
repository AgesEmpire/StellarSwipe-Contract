use soroban_sdk::{Env, Map};
use crate::types::Signal;
use crate::stake::{get_stake_info, StakeInfo, DEFAULT_MINIMUM_STAKE};

/// Maximum adoption count for normalization (cap at 100 adoptions)
const MAX_ADOPTION: u32 = 100;

/// Stake tier thresholds (in stroops, 1 XLM = 10_000_000 stroops)
const BRONZE_THRESHOLD: i128 = 100_000_000;  // 100 XLM (minimum stake)
const SILVER_THRESHOLD: i128 = 500_000_000;  // 500 XLM
const GOLD_THRESHOLD: i128 = 1_000_000_000;  // 1000 XLM

/// Stake tier scores (0-100 scale)
const BRONZE_SCORE: u32 = 33;
const SILVER_SCORE: u32 = 66;
const GOLD_SCORE: u32 = 100;

/// Component weights (must sum to 1.0)
/// Represented as basis points (10000 = 100%)
const SUCCESS_RATE_WEIGHT: u32 = 4000;      // 40%
const ADOPTION_WEIGHT: u32 = 2000;          // 20%
const STAKE_TIER_WEIGHT: u32 = 2000;        // 20%
const AI_SCORE_WEIGHT: u32 = 2000;          // 20%

/// Stake tier enum for classification
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum StakeTier {
    None,
    Bronze,
    Silver,
    Gold,
}

/// Calculate the composite quality score for a signal (0-100)
///
/// Formula: (success_rate * 0.4) + (adoption_normalized * 0.2) + (stake_tier_score * 0.2) + (ai_score * 0.2)
///
/// If AI score is absent, its weight is redistributed to success_rate:
/// Formula: (success_rate * 0.6) + (adoption_normalized * 0.2) + (stake_tier_score * 0.2)
///
/// # Arguments
/// * `env` - Soroban environment
/// * `signal` - The signal to score
///
/// # Returns
/// Quality score from 0 to 100
pub fn calculate_quality_score(env: &Env, signal: &Signal) -> u32 {
    // Calculate success rate (0-100)
    let success_rate = calculate_success_rate(signal);
    
    // Calculate normalized adoption (0-100)
    let adoption_normalized = normalize_adoption(signal.adoption_count);
    
    // Calculate stake tier score (0-100)
    let stake_tier_score = calculate_stake_tier_score(env, &signal.provider);
    
    // Check if AI score is present
    let has_ai_score = signal.ai_validation_score.is_some();
    let ai_score = signal.ai_validation_score.unwrap_or(0);
    
    // Calculate weighted score
    let score = if has_ai_score {
        // All components present: use standard weights
        calculate_weighted_score_with_ai(
            success_rate,
            adoption_normalized,
            stake_tier_score,
            ai_score,
        )
    } else {
        // AI score missing: redistribute its weight to success_rate
        calculate_weighted_score_without_ai(
            success_rate,
            adoption_normalized,
            stake_tier_score,
        )
    };
    
    // Ensure score is within 0-100 range
    score.min(100)
}

/// Calculate success rate from signal executions (0-100)
fn calculate_success_rate(signal: &Signal) -> u32 {
    if signal.executions == 0 {
        return 0;
    }
    
    // Calculate percentage: (successful / total) * 100
    let rate = (signal.successful_executions as u64 * 100) / signal.executions as u64;
    rate as u32
}

/// Normalize adoption count to 0-100 scale
/// Caps at MAX_ADOPTION (100 adoptions = 100 score)
fn normalize_adoption(adoption_count: u32) -> u32 {
    if adoption_count >= MAX_ADOPTION {
        return 100;
    }
    
    // Linear scaling: (adoption_count / MAX_ADOPTION) * 100
    (adoption_count * 100) / MAX_ADOPTION
}

/// Calculate stake tier score based on provider's stake amount
fn calculate_stake_tier_score(env: &Env, provider: &soroban_sdk::Address) -> u32 {
    let stake_info = get_stake_info(env, provider);
    
    let stake_amount = match stake_info {
        Some(info) => info.amount,
        None => 0,
    };
    
    let tier = get_stake_tier(stake_amount);
    get_tier_score(tier)
}

/// Determine stake tier from stake amount
pub fn get_stake_tier(stake_amount: i128) -> StakeTier {
    if stake_amount >= GOLD_THRESHOLD {
        StakeTier::Gold
    } else if stake_amount >= SILVER_THRESHOLD {
        StakeTier::Silver
    } else if stake_amount >= BRONZE_THRESHOLD {
        StakeTier::Bronze
    } else {
        StakeTier::None
    }
}

/// Get score for a stake tier
fn get_tier_score(tier: StakeTier) -> u32 {
    match tier {
        StakeTier::Gold => GOLD_SCORE,
        StakeTier::Silver => SILVER_SCORE,
        StakeTier::Bronze => BRONZE_SCORE,
        StakeTier::None => 0,
    }
}

/// Calculate weighted score when all components (including AI) are present
fn calculate_weighted_score_with_ai(
    success_rate: u32,
    adoption_normalized: u32,
    stake_tier_score: u32,
    ai_score: u32,
) -> u32 {
    let weighted_sum = (success_rate as u64 * SUCCESS_RATE_WEIGHT as u64)
        + (adoption_normalized as u64 * ADOPTION_WEIGHT as u64)
        + (stake_tier_score as u64 * STAKE_TIER_WEIGHT as u64)
        + (ai_score as u64 * AI_SCORE_WEIGHT as u64);
    
    // Divide by 10000 to convert from basis points to percentage
    (weighted_sum / 10000) as u32
}

/// Calculate weighted score when AI component is missing
/// Redistributes AI weight (20%) to success_rate (making it 60% total)
fn calculate_weighted_score_without_ai(
    success_rate: u32,
    adoption_normalized: u32,
    stake_tier_score: u32,
) -> u32 {
    // New weights without AI: success_rate 60%, adoption 20%, stake 20%
    const SUCCESS_RATE_WEIGHT_NO_AI: u32 = 6000; // 60%
    
    let weighted_sum = (success_rate as u64 * SUCCESS_RATE_WEIGHT_NO_AI as u64)
        + (adoption_normalized as u64 * ADOPTION_WEIGHT as u64)
        + (stake_tier_score as u64 * STAKE_TIER_WEIGHT as u64);
    
    // Divide by 10000 to convert from basis points to percentage
    (weighted_sum / 10000) as u32
}

/// Public function to get signal quality score by signal ID
pub fn get_signal_quality_score(env: &Env, signal_id: u64) -> Option<u32> {
    let signals: Map<u64, Signal> = env
        .storage()
        .instance()
        .get(&crate::StorageKey::Signals)?;
    
    let signal = signals.get(signal_id)?;
    Some(calculate_quality_score(env, &signal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as TestAddress, Address, Env, Map, String, Vec};
    use crate::types::{Signal, SignalAction, SignalStatus};
    use crate::categories::{SignalCategory, RiskLevel};
    use crate::stake::StakeInfo;

    fn sdk_string(env: &Env, s: &str) -> String {
        #[allow(deprecated)]
        String::from_slice(env, s)
    }

    fn create_test_signal(
        env: &Env,
        provider: Address,
        executions: u32,
        successful_executions: u32,
        adoption_count: u32,
        ai_score: Option<u32>,
    ) -> Signal {
        Signal {
            id: 1,
            provider,
            asset_pair: sdk_string(env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100_000_000,
            rationale: sdk_string(env, "Test signal"),
            timestamp: 0,
            expiry: 86400,
            status: SignalStatus::Active,
            executions,
            successful_executions,
            total_volume: 0,
            total_roi: 0,
            category: SignalCategory::SWING,
            tags: Vec::new(env),
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
            submitted_at: 0,
            rationale_hash: sdk_string(env, "hash"),
            confidence: 50,
            adoption_count,
            ai_validation_score: ai_score,
            avg_copier_roi_bps: 0,
            copier_closed_count: 0,
        }
    }

    fn setup_stake(env: &Env, provider: &Address, amount: i128) {
        let mut stakes: Map<Address, StakeInfo> = Map::new(env);
        stakes.set(
            provider.clone(),
            StakeInfo {
                amount,
                last_signal_time: 0,
                locked_until: 0,
            },
        );
        env.storage()
            .instance()
            .set(&crate::StorageKey::ProviderStakes, &stakes);
    }

    #[test]
    fn test_all_components_present() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // Setup: 80% success rate, 50 adoptions, GOLD stake, 90 AI score
        setup_stake(&env, &provider, GOLD_THRESHOLD);
        let signal = create_test_signal(&env, provider, 10, 8, 50, Some(90));
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (80 * 0.4) + (50 * 0.2) + (100 * 0.2) + (90 * 0.2)
        //         = 32 + 10 + 20 + 18 = 80
        assert_eq!(score, 80);
    }

    #[test]
    fn test_missing_ai_score() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // Setup: 80% success rate, 50 adoptions, GOLD stake, NO AI score
        setup_stake(&env, &provider, GOLD_THRESHOLD);
        let signal = create_test_signal(&env, provider, 10, 8, 50, None);
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (80 * 0.6) + (50 * 0.2) + (100 * 0.2)
        //         = 48 + 10 + 20 = 78
        assert_eq!(score, 78);
    }

    #[test]
    fn test_zero_success_rate() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // Setup: 0% success rate, 50 adoptions, GOLD stake, 90 AI score
        setup_stake(&env, &provider, GOLD_THRESHOLD);
        let signal = create_test_signal(&env, provider, 10, 0, 50, Some(90));
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (0 * 0.4) + (50 * 0.2) + (100 * 0.2) + (90 * 0.2)
        //         = 0 + 10 + 20 + 18 = 48
        assert_eq!(score, 48);
    }

    #[test]
    fn test_zero_executions() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // Setup: No executions yet, 0 adoptions, BRONZE stake, 50 AI score
        setup_stake(&env, &provider, BRONZE_THRESHOLD);
        let signal = create_test_signal(&env, provider, 0, 0, 0, Some(50));
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (0 * 0.4) + (0 * 0.2) + (33 * 0.2) + (50 * 0.2)
        //         = 0 + 0 + 6.6 + 10 = 16.6 ≈ 16
        assert_eq!(score, 16);
    }

    #[test]
    fn test_max_adoption_capped() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // Setup: 150 adoptions (should cap at 100)
        setup_stake(&env, &provider, SILVER_THRESHOLD);
        let signal = create_test_signal(&env, provider, 10, 10, 150, Some(80));
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (100 * 0.4) + (100 * 0.2) + (66 * 0.2) + (80 * 0.2)
        //         = 40 + 20 + 13.2 + 16 = 89.2 ≈ 89
        assert_eq!(score, 89);
    }

    #[test]
    fn test_stake_tiers() {
        assert_eq!(get_stake_tier(0), StakeTier::None);
        assert_eq!(get_stake_tier(50_000_000), StakeTier::None);
        assert_eq!(get_stake_tier(100_000_000), StakeTier::Bronze);
        assert_eq!(get_stake_tier(300_000_000), StakeTier::Bronze);
        assert_eq!(get_stake_tier(500_000_000), StakeTier::Silver);
        assert_eq!(get_stake_tier(750_000_000), StakeTier::Silver);
        assert_eq!(get_stake_tier(1_000_000_000), StakeTier::Gold);
        assert_eq!(get_stake_tier(2_000_000_000), StakeTier::Gold);
    }

    #[test]
    fn test_tier_scores() {
        assert_eq!(get_tier_score(StakeTier::None), 0);
        assert_eq!(get_tier_score(StakeTier::Bronze), 33);
        assert_eq!(get_tier_score(StakeTier::Silver), 66);
        assert_eq!(get_tier_score(StakeTier::Gold), 100);
    }

    #[test]
    fn test_normalize_adoption() {
        assert_eq!(normalize_adoption(0), 0);
        assert_eq!(normalize_adoption(25), 25);
        assert_eq!(normalize_adoption(50), 50);
        assert_eq!(normalize_adoption(100), 100);
        assert_eq!(normalize_adoption(150), 100); // Capped
    }

    #[test]
    fn test_calculate_success_rate() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // 0 executions
        let signal = create_test_signal(&env, provider.clone(), 0, 0, 0, None);
        assert_eq!(calculate_success_rate(&signal), 0);
        
        // 100% success
        let signal = create_test_signal(&env, provider.clone(), 10, 10, 0, None);
        assert_eq!(calculate_success_rate(&signal), 100);
        
        // 50% success
        let signal = create_test_signal(&env, provider.clone(), 10, 5, 0, None);
        assert_eq!(calculate_success_rate(&signal), 50);
        
        // 75% success
        let signal = create_test_signal(&env, provider.clone(), 8, 6, 0, None);
        assert_eq!(calculate_success_rate(&signal), 75);
    }

    #[test]
    fn test_score_always_0_to_100() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // Test various combinations
        setup_stake(&env, &provider, GOLD_THRESHOLD);
        
        // All max values
        let signal = create_test_signal(&env, provider.clone(), 100, 100, 200, Some(100));
        let score = calculate_quality_score(&env, &signal);
        assert!(score <= 100);
        
        // All min values
        let signal = create_test_signal(&env, provider.clone(), 0, 0, 0, Some(0));
        let score = calculate_quality_score(&env, &signal);
        assert!(score >= 0 && score <= 100);
    }

    #[test]
    fn test_bronze_stake_tier() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        setup_stake(&env, &provider, BRONZE_THRESHOLD);
        let signal = create_test_signal(&env, provider, 10, 8, 50, Some(80));
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (80 * 0.4) + (50 * 0.2) + (33 * 0.2) + (80 * 0.2)
        //         = 32 + 10 + 6.6 + 16 = 64.6 ≈ 64
        assert_eq!(score, 64);
    }

    #[test]
    fn test_silver_stake_tier() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        setup_stake(&env, &provider, SILVER_THRESHOLD);
        let signal = create_test_signal(&env, provider, 10, 8, 50, Some(80));
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (80 * 0.4) + (50 * 0.2) + (66 * 0.2) + (80 * 0.2)
        //         = 32 + 10 + 13.2 + 16 = 71.2 ≈ 71
        assert_eq!(score, 71);
    }

    #[test]
    fn test_no_stake() {
        let env = Env::default();
        let provider = <Address as TestAddress>::generate(&env);
        
        // No stake setup
        let signal = create_test_signal(&env, provider, 10, 8, 50, Some(80));
        
        let score = calculate_quality_score(&env, &signal);
        
        // Expected: (80 * 0.4) + (50 * 0.2) + (0 * 0.2) + (80 * 0.2)
        //         = 32 + 10 + 0 + 16 = 58
        assert_eq!(score, 58);
    }
}
