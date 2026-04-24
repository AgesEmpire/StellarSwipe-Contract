#![cfg(test)]

use crate::types::{SignalEditInput, SignalOutcome};
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

fn edit_price(env: &Env, price: i128) -> SignalEditInput {
    SignalEditInput {
        set_price: true,
        price,
        set_rationale_hash: false,
        rationale_hash: String::from_str(env, ""),
        set_confidence: false,
        confidence: 0,
    }
}

#[test]
fn issue168_update_price_within_window() {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let provider = Address::generate(&env);
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &(env.ledger().timestamp() + 86_400),
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    let edit = edit_price(&env, 2_000_000);
    client.update_signal(&provider, &signal_id, &edit);
    let s = client.get_signal(&signal_id).unwrap();
    assert_eq!(s.price, 2_000_000);
}

#[test]
fn issue168_edit_window_closed() {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let provider = Address::generate(&env);
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &(env.ledger().timestamp() + 86_400),
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    env.ledger().set_timestamp(env.ledger().timestamp() + 61);
    let edit = edit_price(&env, 2_000_000);
    let r = client.try_update_signal(&provider, &signal_id, &edit);
    assert!(r.is_err());
}

#[test]
fn issue168_signal_already_copied() {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);
    let provider = Address::generate(&env);
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &(env.ledger().timestamp() + 86_400),
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    client.increment_adoption(&executor, &signal_id, &1u64);
    let edit = edit_price(&env, 2_000_000);
    let r = client.try_update_signal(&provider, &signal_id, &edit);
    assert!(r.is_err());
}

#[test]
fn issue168_not_owner() {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let provider = Address::generate(&env);
    let attacker = Address::generate(&env);
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &(env.ledger().timestamp() + 86_400),
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    let edit = edit_price(&env, 2_000_000);
    let r = client.try_update_signal(&attacker, &signal_id, &edit);
    assert!(r.is_err());
}



#[test]
fn issue168_field_not_editable_invalid_price_and_rationale() {
    use crate::types::SignalEditInput;
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let provider = Address::generate(&env);
    let tags = Vec::new(&env);
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &1_000_000,
        &String::from_str(&env, "Rationale"),
        &(env.ledger().timestamp() + 86_400),
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    let bad_price = SignalEditInput {
        set_price: true,
        price: 0,
        set_rationale_hash: false,
        rationale_hash: String::from_str(&env, ""),
        set_confidence: false,
        confidence: 0,
    };
    assert!(client
        .try_update_signal(&provider, &signal_id, &bad_price)
        .is_err());
    let bad_rationale = SignalEditInput {
        set_price: false,
        price: 0,
        set_rationale_hash: true,
        rationale_hash: String::from_str(&env, ""),
        set_confidence: false,
        confidence: 0,
    };
    assert!(client
        .try_update_signal(&provider, &signal_id, &bad_rationale)
        .is_err());
}


#[test]
fn issue170_first_profit_updates_reputation() {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);
    let provider = Address::generate(&env);
    let tags = Vec::new(&env);
    let expiry = env.ledger().timestamp() + 10_000;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Rationale"),
        &expiry,
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    env.ledger().set_timestamp(expiry + 1);
    client.cleanup_expired_signals(&100);
    client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Profit);
    let score = client.get_provider_reputation_score(&provider);
    assert_eq!(score, (50u32 * 9 + 100) / 10);
}

#[test]
fn issue170_outcome_twice_fails() {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);
    let provider = Address::generate(&env);
    let tags = Vec::new(&env);
    let expiry = env.ledger().timestamp() + 20_000;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Rationale"),
        &expiry,
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    env.ledger().set_timestamp(expiry + 1);
    client.cleanup_expired_signals(&100);
    client.record_signal_outcome(&executor, &signal_id, &SignalOutcome::Neutral);
    let r = client.try_record_signal_outcome(&executor, &signal_id, &SignalOutcome::Loss);
    assert!(r.is_err());
}

#[test]
fn issue170_non_executor_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let executor = Address::generate(&env);
    client.set_trade_executor(&admin, &executor);
    let rando = Address::generate(&env);
    let provider = Address::generate(&env);
    let tags = Vec::new(&env);
    let expiry = env.ledger().timestamp() + 30_000;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::types::SignalAction::Buy,
        &100_000,
        &String::from_str(&env, "Rationale"),
        &expiry,
        &crate::categories::SignalCategory::SWING,
        &tags,
        &crate::categories::RiskLevel::Medium,
    );
    env.ledger().set_timestamp(expiry + 1);
    client.cleanup_expired_signals(&100);
    let r = client.try_record_signal_outcome(&rando, &signal_id, &SignalOutcome::Profit);
    assert!(r.is_err());
}
