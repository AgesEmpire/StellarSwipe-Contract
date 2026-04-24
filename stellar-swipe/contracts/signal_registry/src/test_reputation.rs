//! Tests for trust score calculation and management

#[cfg(test)]
mod tests {
    use crate::reputation::{
        calculate_trust_score, calculate_stake_component, calculate_tenure_component,
        calculate_weighted_score, get_first_signal_time, get_trust_score, get_trust_score_tier,
        record_first_signal, store_trust_score, update_median_values, ReputationDataKey,
        TrustScoreComponents, TrustScoreDetails, TrustScoreTier,
    };
    use crate::stake::StakeInfo;
    use crate::types::ProviderPerformance;
    use crate::SignalRegistry;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Address, Env, Vec};

    fn setup_env() -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(500_000_000);
        env
    }

    fn with_registry<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = setup_env();
        #[allow(deprecated)]
        let cid = env.register_contract(None, SignalRegistry);
        env.as_contract(&cid, || f(&env))
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
            avg_return: 500,
            total_volume: 1000000,
        }
    }

    #[test]
    fn test_trust_score_insufficient_history() {
        let env = setup_env();
        let provider = create_test_provider(&env);
        let performance = create_test_performance(3, 2, 6667);
        let score_details = calculate_trust_score(&env, &provider, &performance, &None);
        assert_eq!(score_details.score, 0);
        assert_eq!(score_details.tier, TrustScoreTier::NewUnproven);
        assert!(!score_details.has_sufficient_history);
    }

    #[test]
    fn test_trust_score_highly_trusted() {
        with_registry(|env| {
            let provider = create_test_provider(env);
            update_median_values(env, 100_000_000, 50);
            let first_signal_time = env.ledger().timestamp() - (200 * 24 * 60 * 60);
            env.storage()
                .persistent()
                .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &first_signal_time);
            let performance = create_test_performance(20, 18, 9000);
            let stake_info = Some(StakeInfo {
                amount: 300_000_000,
                last_signal_time: env.ledger().timestamp(),
                locked_until: env.ledger().timestamp() + 86400,
            });
            let score_details = calculate_trust_score(env, &provider, &performance, &stake_info);
            assert!(score_details.score >= 65);
            assert!(
                score_details.tier == TrustScoreTier::HighlyTrusted
                    || score_details.tier == TrustScoreTier::Trusted
            );
            assert!(score_details.has_sufficient_history);
            assert!(score_details.components.success_rate == 9000);
            assert!(score_details.components.tenure_normalized > 5000);
        });
    }

    #[test]
    fn test_trust_score_new_provider() {
        let env = setup_env();
        let provider = create_test_provider(&env);
        let performance = ProviderPerformance::default();
        let score_details = calculate_trust_score(&env, &provider, &performance, &None);
        assert_eq!(score_details.score, 0);
        assert_eq!(score_details.tier, TrustScoreTier::NewUnproven);
        assert!(!score_details.has_sufficient_history);
    }

    #[test]
    fn test_trust_score_components_calculation() {
        with_registry(|env| {
            let provider = create_test_provider(env);
            update_median_values(env, 100_000_000, 50);
            let first_signal_time = env.ledger().timestamp() - (100 * 24 * 60 * 60);
            env.storage()
                .persistent()
                .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &first_signal_time);
            let performance = create_test_performance(10, 7, 7000);
            let stake_info = Some(StakeInfo {
                amount: 150_000_000,
                last_signal_time: env.ledger().timestamp(),
                locked_until: env.ledger().timestamp() + 86400,
            });
            let score_details = calculate_trust_score(env, &provider, &performance, &stake_info);
            assert_eq!(score_details.components.success_rate, 7000);
            assert!(score_details.components.tenure_normalized > 2500);
            assert!(score_details.components.stake_normalized > 5000);
            assert!(score_details.score > 40 && score_details.score < 80);
        });
    }

    #[test]
    fn test_trust_score_tier_boundaries() {
        assert_eq!(get_trust_score_tier(79), TrustScoreTier::Trusted);
        assert_eq!(get_trust_score_tier(80), TrustScoreTier::HighlyTrusted);
        assert_eq!(get_trust_score_tier(60), TrustScoreTier::Trusted);
        assert_eq!(get_trust_score_tier(59), TrustScoreTier::Emerging);
        assert_eq!(get_trust_score_tier(40), TrustScoreTier::Emerging);
        assert_eq!(get_trust_score_tier(39), TrustScoreTier::NewUnproven);
    }

    #[test]
    fn test_stake_component_normalization() {
        with_registry(|env| {
            update_median_values(env, 100_000_000, 50);
            let low_stake = Some(StakeInfo {
                amount: 50_000_000,
                last_signal_time: 0,
                locked_until: 0,
            });
            let high_stake = Some(StakeInfo {
                amount: 250_000_000,
                last_signal_time: 0,
                locked_until: 0,
            });
            assert_eq!(calculate_stake_component(env, &low_stake), 5000);
            assert_eq!(calculate_stake_component(env, &high_stake), 10000);
        });
    }

    #[test]
    fn test_tenure_component_calculation() {
        with_registry(|env| {
            let provider = create_test_provider(env);
            let recent_provider = env.ledger().timestamp() - (10 * 24 * 60 * 60);
            let established_provider = env.ledger().timestamp() - (200 * 24 * 60 * 60);
            let veteran_provider = env.ledger().timestamp() - (400 * 24 * 60 * 60);
            env.storage()
                .persistent()
                .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &recent_provider);
            let component_recent =
                calculate_tenure_component(env, &provider, env.ledger().timestamp());
            assert!(component_recent < 1000);
            env.storage()
                .persistent()
                .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &established_provider);
            let component_established =
                calculate_tenure_component(env, &provider, env.ledger().timestamp());
            assert!(component_established > 5000);
            env.storage()
                .persistent()
                .set(&ReputationDataKey::FirstSignalTime(provider.clone()), &veteran_provider);
            let component_veteran =
                calculate_tenure_component(env, &provider, env.ledger().timestamp());
            assert_eq!(component_veteran, 10000);
        });
    }

    #[test]
    fn test_weighted_score_calculation() {
        assert_eq!(calculate_weighted_score(10000, 10000, 10000, 10000, 10000), 100);
        assert_eq!(calculate_weighted_score(5000, 5000, 5000, 5000, 5000), 50);
        assert_eq!(calculate_weighted_score(0, 0, 0, 0, 0), 0);
    }

    #[test]
    fn test_record_first_signal() {
        with_registry(|env| {
            let provider = create_test_provider(env);
            assert_eq!(get_first_signal_time(env, &provider), 0);
            record_first_signal(env, &provider);
            let recorded_time = get_first_signal_time(env, &provider);
            assert_eq!(recorded_time, env.ledger().timestamp());
            env.ledger().set_timestamp(env.ledger().timestamp() + 1000);
            record_first_signal(env, &provider);
            assert_eq!(get_first_signal_time(env, &provider), recorded_time);
        });
    }

    #[test]
    fn test_trust_score_storage_and_retrieval() {
        with_registry(|env| {
            let provider = create_test_provider(env);
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
            store_trust_score(env, &provider, &score_details);
            let retrieved = get_trust_score(env, &provider).unwrap();
            assert_eq!(retrieved.score, score_details.score);
            assert_eq!(retrieved.tier, score_details.tier);
            assert_eq!(
                retrieved.has_sufficient_history,
                score_details.has_sufficient_history
            );
            assert_eq!(
                retrieved.components.success_rate,
                score_details.components.success_rate
            );
        });
    }
}
