#![cfg(test)]

use crate::categories::{RiskLevel, SignalCategory};
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{testutils::Address as TestAddress, Address, Env, String, Vec};

fn setup_env() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    (env, admin, client)
}

fn create_string(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

#[test]
fn test_create_signal_with_category_and_tags() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let mut tags = Vec::new(&env);
    tags.push_back(create_string(&env, "bullish"));
    tags.push_back(create_string(&env, "breakout"));

    let signal_id = client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Strong breakout pattern"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );

    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.category, SignalCategory::SWING);
    assert_eq!(signal.tags.len(), 2);
    assert_eq!(signal.risk_level, RiskLevel::Medium);
}

#[test]
fn test_add_tags_to_signal() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let tags = Vec::new(&env);

    let signal_id = client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Test signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SCALP,
        &tags,
        &RiskLevel::High,
    );

    let mut new_tags = Vec::new(&env);
    new_tags.push_back(create_string(&env, "momentum"));
    new_tags.push_back(create_string(&env, "high-risk"));

    client.add_tags_to_signal(&provider, &signal_id, &new_tags);

    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.tags.len(), 2);
}

#[test]
#[should_panic]
fn test_add_tags_exceeds_max() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let mut initial_tags = Vec::new(&env);
    for i in 0..8 {
        initial_tags.push_back(create_string(&env, &format!("tag{}", i)));
    }

    let signal_id = client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Test signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::DayTrade,
        &initial_tags,
        &RiskLevel::Low,
    );

    let mut new_tags = Vec::new(&env);
    new_tags.push_back(create_string(&env, "tag8"));
    new_tags.push_back(create_string(&env, "tag9"));
    new_tags.push_back(create_string(&env, "tag10"));

    // Should panic - exceeds max 10 tags
    client.add_tags_to_signal(&provider, &signal_id, &new_tags);
}

#[test]
fn test_deduplicate_tags() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let mut tags = Vec::new(&env);
    tags.push_back(create_string(&env, "bullish"));
    tags.push_back(create_string(&env, "breakout"));
    tags.push_back(create_string(&env, "bullish")); // Duplicate

    let signal_id = client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Test signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::ARBITRAGE,
        &tags,
        &RiskLevel::Medium,
    );

    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(signal.tags.len(), 2); // Deduplicated
}

#[test]
fn test_filter_by_category() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    // Create signals with different categories
    let tags = Vec::new(&env);

    client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Swing trade"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Low,
    );

    client.create_signal(
        &provider,
        &create_string(&env, "BTC/USDC"),
        &crate::types::SignalAction::Sell,
        &50_000_000_000,
        &create_string(&env, "Day trade"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SCALP,
        &tags,
        &RiskLevel::High,
    );

    let mut categories = Vec::new(&env);
    categories.push_back(SignalCategory::SwingTrade);

    let filtered = client.get_signals_filtered(&Some(categories), &None, &None, &0, &10);

    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered.get(0).unwrap().category,
        SignalCategory::SwingTrade
    );
}

#[test]
fn test_filter_by_tags() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let mut tags1 = Vec::new(&env);
    tags1.push_back(create_string(&env, "bullish"));

    let mut tags2 = Vec::new(&env);
    tags2.push_back(create_string(&env, "bearish"));

    client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Bullish signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SwingTrade,
        &tags1,
        &RiskLevel::Low,
    );

    client.create_signal(
        &provider,
        &create_string(&env, "BTC/USDC"),
        &crate::types::SignalAction::Sell,
        &50_000_000_000,
        &create_string(&env, "Bearish signal"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::DayTrade,
        &tags2,
        &RiskLevel::High,
    );

    let mut filter_tags = Vec::new(&env);
    filter_tags.push_back(create_string(&env, "bullish"));

    let filtered = client.get_signals_filtered(&None, &Some(filter_tags), &None, &0, &10);

    assert_eq!(filtered.len(), 1);
    assert!(
        filtered.get(0).unwrap().tags.get(0).unwrap().to_bytes()
            == create_string(&env, "bullish").to_bytes()
    );
}

#[test]
fn test_filter_by_risk_level() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let tags = Vec::new(&env);

    client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Low risk"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::LongTerm,
        &tags,
        &RiskLevel::Low,
    );

    client.create_signal(
        &provider,
        &create_string(&env, "BTC/USDC"),
        &crate::types::SignalAction::Sell,
        &50_000_000_000,
        &create_string(&env, "High risk"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::Scalping,
        &tags,
        &RiskLevel::High,
    );

    let mut risk_levels = Vec::new(&env);
    risk_levels.push_back(RiskLevel::High);

    let filtered = client.get_signals_filtered(&None, &None, &Some(risk_levels), &0, &10);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered.get(0).unwrap().risk_level, RiskLevel::High);
}

#[test]
fn test_combined_filters() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let mut tags1 = Vec::new(&env);
    tags1.push_back(create_string(&env, "momentum"));

    let mut tags2 = Vec::new(&env);
    tags2.push_back(create_string(&env, "reversal"));

    // Signal 1: SwingTrade, momentum, Medium
    client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Signal 1"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SwingTrade,
        &tags1,
        &RiskLevel::Medium,
    );

    // Signal 2: DayTrade, reversal, High
    client.create_signal(
        &provider,
        &create_string(&env, "BTC/USDC"),
        &crate::types::SignalAction::Sell,
        &50_000_000_000,
        &create_string(&env, "Signal 2"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::DayTrade,
        &tags2,
        &RiskLevel::High,
    );

    let mut categories = Vec::new(&env);
    categories.push_back(SignalCategory::SwingTrade);

    let mut filter_tags = Vec::new(&env);
    filter_tags.push_back(create_string(&env, "momentum"));

    let mut risk_levels = Vec::new(&env);
    risk_levels.push_back(RiskLevel::Medium);

    let filtered = client.get_signals_filtered(
        &Some(categories),
        &Some(filter_tags),
        &Some(risk_levels),
        &0,
        &10,
    );

    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered.get(0).unwrap().category,
        SignalCategory::SwingTrade
    );
}

#[test]
fn test_popular_tags() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    // Create multiple signals with overlapping tags
    let mut tags1 = Vec::new(&env);
    tags1.push_back(create_string(&env, "bullish"));
    tags1.push_back(create_string(&env, "breakout"));

    let mut tags2 = Vec::new(&env);
    tags2.push_back(create_string(&env, "bullish"));
    tags2.push_back(create_string(&env, "momentum"));

    client.create_signal(
        &provider,
        &create_string(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &create_string(&env, "Signal 1"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::SwingTrade,
        &tags1,
        &RiskLevel::Low,
    );

    client.create_signal(
        &provider,
        &create_string(&env, "BTC/USDC"),
        &crate::types::SignalAction::Buy,
        &50_000_000_000,
        &create_string(&env, "Signal 2"),
        &(env.ledger().timestamp() + 86400),
        &SignalCategory::Momentum,
        &tags2,
        &RiskLevel::Medium,
    );

    let popular = client.get_popular_tags(&10);

    assert!(popular.len() > 0);
    // "bullish" should be most popular (used twice)
    let top_tag = popular.get(0).unwrap();
    assert_eq!(top_tag.1, 2); // Count should be 2
}

#[test]
fn test_suggest_tags() {
    let (env, _admin, _client) = setup_env();

    let rationale = create_string(
        &env,
        "Strong breakout above resistance with bullish momentum",
    );
    let suggestions = crate::categories::auto_suggest_tags(&env, &rationale);

    assert!(suggestions.len() > 0);
    // Should suggest "breakout", "bullish", "momentum"
}

#[test]
fn test_pagination() {
    let (env, _admin, client) = setup_env();
    let provider = Address::generate(&env);

    let tags = Vec::new(&env);

    // Create 5 signals
    for i in 0..5 {
        client.create_signal(
            &provider,
            &create_string(&env, "XLM/USDC"),
            &crate::types::SignalAction::Buy,
            &(1_000_000 + i as i128),
            &create_string(&env, &format!("Signal {}", i)),
            &(env.ledger().timestamp() + 86400),
            &SignalCategory::SwingTrade,
            &tags,
            &RiskLevel::Low,
        );
    }

    // Get first 2
    let page1 = client.get_signals_filtered(&None, &None, &None, &0, &2);
    assert_eq!(page1.len(), 2);

    // Get next 2
    let page2 = client.get_signals_filtered(&None, &None, &None, &2, &2);
    assert_eq!(page2.len(), 2);

    // Get last 1
    let page3 = client.get_signals_filtered(&None, &None, &None, &4, &2);
    assert_eq!(page3.len(), 1);
}
