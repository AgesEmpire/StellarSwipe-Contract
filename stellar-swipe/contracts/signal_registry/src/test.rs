#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, Env, vec, String};
use crate::categories::{SignalCategory, RiskLevel};

#[test]
fn test_initialize_and_admin() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let stored_admin = client.get_admin();
    assert_eq!(stored_admin, admin);
}

#[test]
fn test_admin_cannot_initialize_twice() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    client.initialize(&admin1);
    let result = client.try_initialize(&admin2);
    assert!(result.is_err());
}

#[test]
fn create_and_read_signal() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 60;

    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Breakout confirmed"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );

    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.id, signal_id);
    assert_eq!(signal.status, SignalStatus::Active);
}

#[test]
fn test_invalid_asset_pair_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 60;

    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLMUSDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );
    assert!(result.is_err());

    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/XLM"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );
    assert!(result.is_err());

    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC:INVALID"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );
    assert!(result.is_err());
}

#[test]
fn test_custom_asset_pair_with_issuer() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 60;

    let signal_id = client.create_signal(
        &provider,
        &String::from_str(
            &env,
            "XLM/USDC:GDUKMGUGDZQK6YHYA5Z6AY2G4XDSZPSZ3SW5UN3ARVMO6QSRDWP5YLEX",
        ),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Full format"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );

    assert!(signal_id > 0);
}

#[test]
fn test_pause_blocks_signals() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 60;

    // Pause trading
    client.pause_trading(&admin);
    assert!(client.is_paused());

    // Try to create signal - should fail
    let result = client.try_create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );

    assert!(result.is_err());

    // Unpause
    client.unpause_trading(&admin);
    assert!(!client.is_paused());

    // Now should work
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );

    assert!(signal_id > 0);
}

#[test]
fn test_pause_auto_expires() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Pause trading
    client.pause_trading(&admin);
    assert!(client.is_paused());

    // Move time forward past 48 hours
    use soroban_sdk::testutils::Ledger;
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + 48 * 60 * 60 + 1);

    // Should be auto-unpaused
    assert!(!client.is_paused());
}

#[test]
fn test_admin_config_updates() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Update min stake
    client.set_min_stake(&admin, &200_000_000);

    // Update trade fee
    client.set_trade_fee(&admin, &20);

    // Update risk defaults
    client.set_risk_defaults(&admin, &20, &25);

    let config = client.get_config();
    assert_eq!(config.min_stake, 200_000_000);
    assert_eq!(config.trade_fee_bps, 20);
    assert_eq!(config.default_stop_loss, 20);
    assert_eq!(config.default_position_limit, 25);
}

#[test]
fn test_unauthorized_admin_actions() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);

    client.initialize(&admin);

    // Attacker tries to update min stake
    let result = client.try_set_min_stake(&attacker, &500_000_000);
    assert!(result.is_err());

    // Attacker tries to pause
    let result = client.try_pause_trading(&attacker);
    assert!(result.is_err());

    // Attacker tries to transfer admin
    let new_admin = Address::generate(&env);
    let result = client.try_transfer_admin(&attacker, &new_admin);
    assert!(result.is_err());
}

#[test]
fn test_transfer_admin() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    client.initialize(&admin1);
    client.transfer_admin(&admin1, &admin2);

    let current_admin = client.get_admin();
    assert_eq!(current_admin, admin2);

    // Old admin should no longer work
    let result = client.try_pause_trading(&admin1);
    assert!(result.is_err());

    // New admin should work
    client.pause_trading(&admin2);
    assert!(client.is_paused());
}

#[test]
fn test_multisig_enable_and_use() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signer3 = Address::generate(&env);

    client.initialize(&admin);

    let mut signers = Vec::new(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());
    signers.push_back(signer3.clone());

    // Enable multi-sig with 2-of-3 threshold
    client.enable_multisig(&admin, &signers, &2);

    assert!(client.is_multisig_enabled());
    assert_eq!(client.get_multisig_threshold(), 2);

    let returned_signers = client.get_multisig_signers();
    assert_eq!(returned_signers.len(), 3);

    // Any signer should be able to pause
    client.pause_trading(&signer1);
    assert!(client.is_paused());
}

#[test]
fn test_multisig_add_remove_signers() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let signer4 = Address::generate(&env);

    client.initialize(&admin);

    let mut signers = Vec::new(&env);
    signers.push_back(admin.clone());
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());

    client.enable_multisig(&admin, &signers, &2);

    assert_eq!(client.get_multisig_signers().len(), 3);

    // Add one more, then we can remove
    client.add_multisig_signer(&admin, &signer4);
    client.remove_multisig_signer(&admin, &signer1);
    assert_eq!(client.get_multisig_signers().len(), 3);
}

#[test]
fn test_invalid_parameter_updates() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Invalid min stake (negative)
    let result = client.try_set_min_stake(&admin, &-100);
    assert!(result.is_err());

    // Invalid trade fee (> 100 bps)
    let result = client.try_set_trade_fee(&admin, &150);
    assert!(result.is_err());

    // Invalid risk parameters (> 100%)
    let result = client.try_set_risk_defaults(&admin, &150, &20);
    assert!(result.is_err());
}

#[test]
fn provider_stats_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let expiry = env.ledger().timestamp() + 120;

    client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/BTC"),
        &SignalAction::Sell,
        &200_000,
        &String::from_str(&env, "Resistance hit"),
        &expiry,
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );

    let stats = client.get_provider_stats(&provider).unwrap();
    assert_eq!(stats.total_copies, 0);
}

#[test]
fn test_fee_calculation_and_collection() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Set platform treasury
    let platform_treasury = Address::generate(&env);
    client.set_platform_treasury(&admin, &platform_treasury);

    // Preview fee for 1000 XLM trade
    let breakdown = client.calculate_fee_preview(&1_000_000_000);

    assert_eq!(breakdown.total_fee, 1_000_000); // 1 XLM (0.1%)
    assert_eq!(breakdown.platform_fee, 700_000); // 0.7 XLM (70%)
    assert_eq!(breakdown.provider_fee, 300_000); // 0.3 XLM (30%)
    assert_eq!(breakdown.trade_amount_after_fee, 999_000_000); // 999 XLM
}

#[test]
fn test_minimum_trade_enforcement() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Trade below minimum should fail
    let result = client.try_calculate_fee_preview(&999);
    assert!(result.is_err());

    // Trade at minimum should work
    let result = client.try_calculate_fee_preview(&1000);
    assert!(result.is_ok());
}

#[test]
fn test_platform_treasury_management() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // No treasury set initially
    assert_eq!(client.get_platform_treasury(), None);

    // Set treasury
    let treasury = Address::generate(&env);
    client.set_platform_treasury(&admin, &treasury);
    assert_eq!(client.get_platform_treasury(), Some(treasury));
}

#[test]
fn test_unauthorized_treasury_update() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let treasury = Address::generate(&env);

    client.initialize(&admin);

    // Non-admin cannot set treasury
    let result = client.try_set_platform_treasury(&attacker, &treasury);
    assert!(result.is_err());
}

#[test]
fn test_get_active_signals_excludes_expired() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Set a known timestamp
    use soroban_sdk::testutils::Ledger;
    env.ledger().set_timestamp(10000);

    let provider = Address::generate(&env);
    let current_time = env.ledger().timestamp();

    // Create 3 active signals
    for _i in 0..3 {
        client.create_signal(
            &provider,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100_000,
            &String::from_str(&env, "Active"),
            &(current_time + 10000),
            &SignalCategory::SWING,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
    }

    // Create 2 expired signals
    for _i in 0..2 {
        client.create_signal(
            &provider,
            &String::from_str(&env, "XLM/BTC"),
            &SignalAction::Sell,
            &200_000,
            &String::from_str(&env, "Expired"),
            &(current_time + 10),
            &SignalCategory::SWING,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
    }

    // Move time to expire the second batch
    env.ledger().set_timestamp(current_time + 100);

    // Get active signals - should only return 3 (followed_only = false)
    let any_user = Address::generate(&env);
    let active = client.get_active_signals(&any_user, &false);
    assert_eq!(active.len(), 3);

    // All returned signals should be active
    for i in 0..active.len() {
        let signal = active.get(i).unwrap();
        assert_eq!(signal.rationale, String::from_str(&env, "Active"));
    }
}

#[test]
fn test_cleanup_batch_limit() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Set a known timestamp
    use soroban_sdk::testutils::Ledger;
    env.ledger().set_timestamp(10000);

    let provider = Address::generate(&env);
    let current_time = env.ledger().timestamp();

    // Create 150 expired signals
    for _ in 0..150 {
        client.create_signal(
            &provider,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100_000,
            &String::from_str(&env, "Test"),
            &(current_time + 10),
            &SignalCategory::SWING,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
    }

    // Move time to expire all
    env.ledger().set_timestamp(current_time + 100);

    // Cleanup with limit of 50
    let (processed, expired) = client.cleanup_expired_signals(&50);

    assert_eq!(processed, 50);
    assert_eq!(expired, 50);

    // Run again to process more
    let (processed2, expired2) = client.cleanup_expired_signals(&50);
    assert_eq!(processed2, 50);
    assert_eq!(expired2, 50);
}

#[test]
fn test_pending_expiry_count() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Set a known timestamp
    use soroban_sdk::testutils::Ledger;
    env.ledger().set_timestamp(10000);

    let provider = Address::generate(&env);
    let current_time = env.ledger().timestamp();

    // Create signals that will be past expiry
    for _i in 0..4 {
        client.create_signal(
            &provider,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100_000,
            &String::from_str(&env, "Test"),
            &(current_time + 10),
            &SignalCategory::SWING,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
    }

    // Initially no pending expiry
    assert_eq!(client.get_pending_expiry_count(), 0);

    // Move time forward
    env.ledger().set_timestamp(current_time + 100);

    // Now should have 4 pending expiry
    assert_eq!(client.get_pending_expiry_count(), 4);

    // After cleanup, none pending
    client.cleanup_expired_signals(&10);
    assert_eq!(client.get_pending_expiry_count(), 0);
}

// ========================================
// Social / Follow Tests
// ========================================

#[test]
fn test_follow_provider() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let user = Address::generate(&env);
    let provider = Address::generate(&env);

    client.follow_provider(&user, &provider);
    assert_eq!(client.get_follower_count(&provider), 1);

    let followed = client.get_followed_providers(&user);
    assert_eq!(followed.len(), 1);
    assert_eq!(followed.get(0).unwrap(), provider);
}

#[test]
fn test_follow_idempotent() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let user = Address::generate(&env);
    let provider = Address::generate(&env);

    client.follow_provider(&user, &provider);
    client.follow_provider(&user, &provider); // idempotent
    assert_eq!(client.get_follower_count(&provider), 1);
}

#[test]
fn test_cannot_follow_self() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let user = Address::generate(&env);
    let result = client.try_follow_provider(&user, &user);
    assert!(result.is_err());
}

#[test]
fn test_unfollow_provider() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let user = Address::generate(&env);
    let provider = Address::generate(&env);

    client.follow_provider(&user, &provider);
    assert_eq!(client.get_follower_count(&provider), 1);

    client.unfollow_provider(&user, &provider);
    assert_eq!(client.get_follower_count(&provider), 0);
    assert_eq!(client.get_followed_providers(&user).len(), 0);
}

#[test]
fn test_unfollow_not_following_no_error() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let user = Address::generate(&env);
    let provider = Address::generate(&env);

    client.unfollow_provider(&user, &provider); // no error
}

#[test]
fn test_feed_filtered_by_followed() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    use soroban_sdk::testutils::Ledger;
    env.ledger().set_timestamp(10000);
    let current_time = env.ledger().timestamp();

    let provider_a = Address::generate(&env);
    let provider_b = Address::generate(&env);
    let user = Address::generate(&env);

    // Create signals from both providers
    client.create_signal(
        &provider_a,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "A1"),
        &(current_time + 10000),
        &SignalCategory::SWING,
        &vec![&env, String::from_str(&env, "A")],
        &RiskLevel::Medium,
    );
    client.create_signal(
        &provider_b,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "B1"),
        &(current_time + 10000),
        &SignalCategory::SCALP,
        &vec![&env, String::from_str(&env, "B")],
        &RiskLevel::High,
    );

    // User follows only provider_a
    client.follow_provider(&user, &provider_a);

    // All signals (followed_only = false)
    let all_active = client.get_active_signals(&user, &false);
    assert_eq!(all_active.len(), 2);

    // Filtered feed (followed_only = true) - only provider_a
    let followed_active = client.get_active_signals(&user, &true);
    assert_eq!(followed_active.len(), 1);
    assert_eq!(followed_active.get(0).unwrap().provider, provider_a);
}

#[test]
fn test_follow_multiple_providers_follower_counts() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let user = Address::generate(&env);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    let p3 = Address::generate(&env);

    client.follow_provider(&user, &p1);
    client.follow_provider(&user, &p2);
    client.follow_provider(&user, &p3);

    assert_eq!(client.get_follower_count(&p1), 1);
    assert_eq!(client.get_follower_count(&p2), 1);
    assert_eq!(client.get_follower_count(&p3), 1);

    let followed = client.get_followed_providers(&user);
    assert_eq!(followed.len(), 3);

    // Unfollow 2
    client.unfollow_provider(&user, &p1);
    client.unfollow_provider(&user, &p2);

    assert_eq!(client.get_follower_count(&p1), 0);
    assert_eq!(client.get_follower_count(&p2), 0);
    assert_eq!(client.get_follower_count(&p3), 1);

    let followed_after = client.get_followed_providers(&user);
    assert_eq!(followed_after.len(), 1);
    assert_eq!(followed_after.get(0).unwrap(), p3);
}

fn build_vars(env: &Env, entries: &[(&str, &str)]) -> Map<String, String> {
    let mut vars = Map::new(env);
    for (k, v) in entries {
        vars.set(String::from_str(env, k), String::from_str(env, v));
    }
    vars
}

#[test]
fn test_create_template_with_variables() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let template_id = client.create_template(
        &provider,
        &String::from_str(&env, "Daily BTC Analysis"),
        &Some(String::from_str(&env, "BTC/USDC")),
        &String::from_str(
            &env,
            "BTC technical analysis for {date}. Entry at {price}, target {target}.",
        ),
    );

    let template = client.get_template(&template_id).unwrap();
    assert_eq!(template.id, template_id);
    assert_eq!(template.provider, provider);
    assert_eq!(
        template.asset_pair,
        Some(String::from_str(&env, "BTC/USDC"))
    );
    assert_eq!(template.use_count, 0);
}

#[test]
fn test_submit_signal_from_template_with_variables() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let template_id = client.create_template(
        &provider,
        &String::from_str(&env, "Quick Long"),
        &Some(String::from_str(&env, "XLM/USDC")),
        &String::from_str(&env, "Buy setup at {price}, target {target}"),
    );

    let vars = build_vars(
        &env,
        &[("action", "buy"), ("price", "101000"), ("target", "120000")],
    );

    let signal_id = client.submit_from_template(&provider, &template_id, &vars);
    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.provider, provider);
    assert_eq!(signal.asset_pair, String::from_str(&env, "XLM/USDC"));
    assert_eq!(signal.action, SignalAction::Buy);
    assert_eq!(signal.price, 101000);
    assert_eq!(
        signal.rationale,
        String::from_str(&env, "Buy setup at 101000, target 120000")
    );

    let template = client.get_template(&template_id).unwrap();
    assert_eq!(template.use_count, 1);
}

#[test]
fn test_submit_signal_from_template_missing_variables_should_error() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let template_id = client.create_template(
        &provider,
        &String::from_str(&env, "Missing Vars"),
        &Some(String::from_str(&env, "XLM/USDC")),
        &String::from_str(&env, "Entry {price}, stop {stop_loss}"),
    );

    let vars = build_vars(&env, &[("action", "buy"), ("price", "100000")]);
    let result = client.try_submit_from_template(&provider, &template_id, &vars);
    assert!(result.is_err());
}

#[test]
fn test_share_template_and_submit_from_another_provider() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let owner = Address::generate(&env);
    let other_provider = Address::generate(&env);

    let template_id = client.create_template(
        &owner,
        &String::from_str(&env, "Shared Template"),
        &None,
        &String::from_str(&env, "Momentum on {asset_pair} at {price}"),
    );

    // Private template cannot be used by another provider
    let private_vars = build_vars(
        &env,
        &[
            ("asset_pair", "BTC/USDC"),
            ("action", "sell"),
            ("price", "90000"),
        ],
    );
    let private_result =
        client.try_submit_from_template(&other_provider, &template_id, &private_vars);
    assert!(private_result.is_err());

    // Share publicly and submit successfully from another provider
    client.set_template_public(&owner, &template_id, &true);
    let signal_id = client.submit_from_template(&other_provider, &template_id, &private_vars);
    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.provider, other_provider);
    assert_eq!(signal.asset_pair, String::from_str(&env, "BTC/USDC"));
    assert_eq!(signal.action, SignalAction::Sell);
}
