//! Tests for trust score calculation and management

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reputation::{
        calculate_stake_component, calculate_tenure_component, calculate_trust_score,
        calculate_weighted_score, get_first_signal_time, get_trust_score, get_trust_score_tier,
        record_first_signal, store_trust_score, update_median_values, ReputationDataKey,
        TrustScoreComponents, TrustScoreDetails, TrustScoreTier,
    };
    use crate::stake::StakeInfo;
    use crate::types::ProviderPerformance;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{vec, Address, Env, Vec};

    fn setup_env() -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(1000000); // Set a base timestamp
        env
    }

    fn create_test_provider(env: &Env) -> Address {
        Address::generate(env)
    }

    fn create_test_performance(
        total_signals: u32,
        successful_signals: u32,
        success_rate: u32,
    ) -> ProviderPerformance {
        ProviderPerformance {
            total_signals,
            successful_signals,
            failed_signals: total_signals - successful_signals,
            total_copies: (total_signals * 10) as u64,
            success_rate,
            avg_return: 500, // 5% average return
            total_volume: 1000000,
        }
    }

    #[test]
    fn test_trust_score_insufficient_history() {
        let env = setup_env();
        let provider = create_test_provider(&env);

        // Provider with only 3 signals (below minimum 5)
        let performance = create_test_performance(3, 2, 6667); // 66.67% success rate

        let score_details = calculate_trust_score(&env, &provider, &performance, &None);

        assert_eq!(score_details.score, 0);
        assert_eq!(score_details.tier, TrustScoreTier::NewUnproven);
        assert!(!score_details.has_sufficient_history);
    }

    #[test]
    fn test_trust_score_highly_trusted() {
        let env = setup_env();
        let provider = create_test_provider(&env);

        // Set up median values
        update_median_values(&env, 100_000_000, 50); // 100 XLM median stake, 50 median followers

        // Record first signal 200 days ago
        let first_signal_time = env.ledger().timestamp() - (200 * 24 * 60 * 60);
        env.storage()
            .persistent()
            .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &first_signal_time);

        // High-performing provider
        let performance = create_test_performance(20, 18, 9000); // 90% success rate

        // High stake and followers
        let stake_info = Some(StakeInfo {
            amount: 300_000_000, // 300 XLM (3x median)
            last_signal_time: env.ledger().timestamp(),
            locked_until: env.ledger().timestamp() + 86400,
        });

        // Mock follower count (would normally be in social module)
        // For this test, we'll assume 150 followers (3x median)

        let score_details = calculate_trust_score(&env, &provider, &performance, &stake_info);

        assert!(score_details.score >= 80); // Should be highly trusted
        assert_eq!(score_details.tier, TrustScoreTier::HighlyTrusted);
        assert!(score_details.has_sufficient_history);

        // Check components are reasonable
        assert!(score_details.components.success_rate == 9000); // 90%
        assert!(score_details.components.tenure_normalized > 5000); // >50% for 200 days
    }

    #[test]
    fn test_trust_score_new_provider() {
        let env = setup_env();
        let provider = create_test_provider(&env);

        // Brand new provider with 0 signals
        let performance = ProviderPerformance::default();

        let score_details = calculate_trust_score(&env, &provider, &performance, &None);

        assert_eq!(score_details.score, 0);
        assert_eq!(score_details.tier, TrustScoreTier::NewUnproven);
        assert!(!score_details.has_sufficient_history);
    }

    #[test]
    fn test_trust_score_components_calculation() {
        let env = setup_env();
        let provider = create_test_provider(&env);

        // Set median values
        update_median_values(&env, 100_000_000, 50);

        // Set first signal time
        let first_signal_time = env.ledger().timestamp() - (100 * 24 * 60 * 60); // 100 days ago
        env.storage()
            .persistent()
            .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &first_signal_time);

        let performance = create_test_performance(10, 7, 7000); // 70% success rate

        let stake_info = Some(StakeInfo {
            amount: 150_000_000, // 150 XLM (1.5x median)
            last_signal_time: env.ledger().timestamp(),
            locked_until: env.ledger().timestamp() + 86400,
        });

        let score_details = calculate_trust_score(&env, &provider, &performance, &stake_info);

        // Verify components
        assert_eq!(score_details.components.success_rate, 7000); // 70%
        assert!(score_details.components.tenure_normalized > 2500); // ~27% for 100 days
        assert!(score_details.components.stake_normalized > 5000); // >50% for 1.5x median

        // Overall score should be reasonable
        assert!(score_details.score > 40 && score_details.score < 80); // Emerging to Trusted range
    }

    #[test]
    fn test_trust_score_tier_boundaries() {
        // Test tier boundaries
        assert_eq!(get_trust_score_tier(79), TrustScoreTier::Trusted);
        assert_eq!(get_trust_score_tier(80), TrustScoreTier::HighlyTrusted);
        assert_eq!(get_trust_score_tier(60), TrustScoreTier::Trusted);
        assert_eq!(get_trust_score_tier(59), TrustScoreTier::Emerging);
        assert_eq!(get_trust_score_tier(40), TrustScoreTier::Emerging);
        assert_eq!(get_trust_score_tier(39), TrustScoreTier::NewUnproven);
    }

    #[test]
    fn test_stake_component_normalization() {
        let env = setup_env();

        // Set median stake
        update_median_values(&env, 100_000_000, 50);

        // Test various stake amounts
        let low_stake = Some(StakeInfo {
            amount: 50_000_000, // 0.5x median
            last_signal_time: 0,
            locked_until: 0,
        });

        let high_stake = Some(StakeInfo {
            amount: 250_000_000, // 2.5x median (capped at 2x)
            last_signal_time: 0,
            locked_until: 0,
        });

        let component_low = calculate_stake_component(&env, &low_stake);
        let component_high = calculate_stake_component(&env, &high_stake);

        assert_eq!(component_low, 5000); // 50% score for 0.5x median
        assert_eq!(component_high, 10000); // 100% score (capped at 2x median)
    }

    #[test]
    fn test_tenure_component_calculation() {
        let env = setup_env();
        let provider = create_test_provider(&env);

        // Test various tenure periods
        let recent_provider = env.ledger().timestamp() - (10 * 24 * 60 * 60); // 10 days
        let established_provider = env.ledger().timestamp() - (200 * 24 * 60 * 60); // 200 days
        let veteran_provider = env.ledger().timestamp() - (400 * 24 * 60 * 60); // 400 days (capped)

        env.storage()
            .persistent()
            .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &recent_provider);

        let component_recent = calculate_tenure_component(&env, &provider, env.ledger().timestamp());
        assert!(component_recent < 1000); // <10% for 10 days

        env.storage()
            .persistent()
            .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &established_provider);

        let component_established = calculate_tenure_component(&env, &provider, env.ledger().timestamp());
        assert!(component_established > 5000); // >50% for 200 days

        env.storage()
            .persistent()
            .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &veteran_provider);

        let component_veteran = calculate_tenure_component(&env, &provider, env.ledger().timestamp());
        assert_eq!(component_veteran, 10000); // 100% (capped at 365 days)
    }

    #[test]
    fn test_weighted_score_calculation() {
        // Test the weighted formula directly
        let score = calculate_weighted_score(10000, 10000, 10000, 10000, 10000);
        assert_eq!(score, 100); // All components 100% = 100 score

        let score = calculate_weighted_score(5000, 5000, 5000, 5000, 5000);
        assert_eq!(score, 50); // All components 50% = 50 score

        let score = calculate_weighted_score(0, 0, 0, 0, 0);
        assert_eq!(score, 0); // All components 0% = 0 score
    }

    #[test]
    fn test_record_first_signal() {
        let env = setup_env();
        let provider = create_test_provider(&env);

        // Initially no first signal time
        let initial_time = get_first_signal_time(&env, &provider);
        assert_eq!(initial_time, 0);

        // Record first signal
        record_first_signal(&env, &provider);

        // Should now have a timestamp
        let recorded_time = get_first_signal_time(&env, &provider);
        assert_eq!(recorded_time, env.ledger().timestamp());

        // Recording again should not change it
        env.ledger().set_timestamp(env.ledger().timestamp() + 1000);
        record_first_signal(&env, &provider);

        let second_time = get_first_signal_time(&env, &provider);
        assert_eq!(second_time, recorded_time); // Should remain the same
    }

    #[test]
    fn test_trust_score_storage_and_retrieval() {
        let env = setup_env();
        let provider = create_test_provider(&env);

        // Create a trust score
        let score_details = TrustScoreDetails {
            score: 75,
            tier: TrustScoreTier::Trusted,
            components: TrustScoreComponents {
                success_rate: 8000,
                consistency: 7000,
                stake_normalized: 6000,
                followers_normalized: 5000,
                tenure_normalized: 9000,
            },
            has_sufficient_history: true,
            last_updated: env.ledger().timestamp(),
        };

        // Store it
        store_trust_score(&env, &provider, &score_details);

        // Retrieve it
        let retrieved = get_trust_score(&env, &provider).unwrap();

        assert_eq!(retrieved.score, score_details.score);
        assert_eq!(retrieved.tier, score_details.tier);
        assert_eq!(retrieved.has_sufficient_history, score_details.has_sufficient_history);
        assert_eq!(retrieved.components.success_rate, score_details.components.success_rate);
    }
}