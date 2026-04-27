//! Integration: SignalRegistry PREMIUM visibility calls this crate's `check_subscription`.

use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use signal_registry::{RiskLevel, SignalAction, SignalCategory, SignalRegistry, SignalRegistryClient};
use user_portfolio::{UserPortfolio, UserPortfolioClient};

#[test]
fn premium_signal_visible_only_to_subscriber_or_provider() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let subscriber = Address::generate(&env);
    let stranger = Address::generate(&env);

    let oracle = Address::generate(&env);
    #[allow(deprecated)]
    let portfolio_id = env.register_contract(None, UserPortfolio);
    let portfolio = UserPortfolioClient::new(&env, &portfolio_id);
    portfolio.initialize(&admin, &oracle);

    let token_admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(token_admin).address();
    StellarAssetClient::new(&env, &token).mint(&subscriber, &50_000_000i128);

    portfolio
        .try_set_provider_subscription_terms(&provider, &token, &100_000i128)
        .unwrap();
    portfolio
        .try_subscribe_to_provider(&subscriber, &provider, &30u32)
        .unwrap();

    #[allow(deprecated)]
    let registry_id = env.register_contract(None, SignalRegistry);
    let registry = SignalRegistryClient::new(&env, &registry_id);
    registry.initialize(&admin);
    registry.set_user_portfolio(&admin, &portfolio_id);

    let tags = Vec::new(&env);
    let signal_id = registry.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Premium note"),
        &(env.ledger().timestamp() + 86_400),
        &SignalCategory::PREMIUM,
        &tags,
        &RiskLevel::Medium,
    );

    assert!(registry
        .get_signal_for_viewer(&signal_id, &stranger)
        .is_none());
    assert!(registry
        .get_signal_for_viewer(&signal_id, &subscriber)
        .is_some());
    assert!(registry
        .get_signal_for_viewer(&signal_id, &provider)
        .is_some());
}

#[test]
fn non_premium_signal_visible_to_any_viewer() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let stranger = Address::generate(&env);

    #[allow(deprecated)]
    let registry_id = env.register_contract(None, SignalRegistry);
    let registry = SignalRegistryClient::new(&env, &registry_id);
    registry.initialize(&admin);

    let tags = Vec::new(&env);
    let signal_id = registry.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Public swing"),
        &(env.ledger().timestamp() + 86_400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Low,
    );

    assert!(registry
        .get_signal_for_viewer(&signal_id, &stranger)
        .is_some());
}
