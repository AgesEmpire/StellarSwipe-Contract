#![cfg(test)]
//! Tests for Issue #607: contract-level feature flag registry.

extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::{
    errors::ContractError,
    feature_flags::{FEAT_COPY_TRADE, FEAT_DCA},
    TradeExecutorContract, TradeExecutorContractClient,
};

fn setup(env: &Env) -> (TradeExecutorContractClient, Address) {
    let admin = Address::generate(env);
    let contract_id = env.register_contract(None, TradeExecutorContract);
    let client = TradeExecutorContractClient::new(env, &contract_id);
    client.initialize(&admin);
    (client, admin)
}

// ── Default behaviour ─────────────────────────────────────────────────────────

#[test]
fn flags_default_to_enabled() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    assert!(client.is_feature_enabled(&String::from_str(&env, FEAT_COPY_TRADE)));
    assert!(client.is_feature_enabled(&String::from_str(&env, FEAT_DCA)));
    assert!(client.is_feature_enabled(&String::from_str(&env, "unknown_flag")));
}

// ── Admin sets a flag ─────────────────────────────────────────────────────────

#[test]
fn admin_can_disable_and_reenable_flag() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    client.set_feature_flag(&String::from_str(&env, FEAT_COPY_TRADE), &false);
    assert!(!client.is_feature_enabled(&String::from_str(&env, FEAT_COPY_TRADE)));

    client.set_feature_flag(&String::from_str(&env, FEAT_COPY_TRADE), &true);
    assert!(client.is_feature_enabled(&String::from_str(&env, FEAT_COPY_TRADE)));
}

// ── Toggling one flag does not affect the other ───────────────────────────────

#[test]
fn disabling_copy_trade_does_not_affect_dca_flag() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    client.set_feature_flag(&String::from_str(&env, FEAT_COPY_TRADE), &false);

    // DCA flag is unaffected.
    assert!(client.is_feature_enabled(&String::from_str(&env, FEAT_DCA)));
}

#[test]
fn disabling_dca_does_not_affect_copy_trade_flag() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    client.set_feature_flag(&String::from_str(&env, FEAT_DCA), &false);

    assert!(client.is_feature_enabled(&String::from_str(&env, FEAT_COPY_TRADE)));
}

// ── Flag state change events ──────────────────────────────────────────────────

#[test]
fn flag_change_emits_event() {
    use soroban_sdk::testutils::Events;
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    client.set_feature_flag(&String::from_str(&env, FEAT_COPY_TRADE), &false);

    // At least one event should be published.
    assert!(!env.events().all().is_empty());
}

// ── execute_copy_trade blocked when flag disabled ─────────────────────────────

#[test]
fn execute_copy_trade_blocked_when_flag_disabled() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    client.set_feature_flag(&String::from_str(&env, FEAT_COPY_TRADE), &false);

    let user = Address::generate(&env);
    let token = Address::generate(&env);

    let result = client.try_execute_copy_trade(
        &user,
        &token,
        &1_000_000i128,
        &None,
        &crate::OrderType::Market,
        &None,
        &1u64,
        &soroban_sdk::Bytes::from_array(&env, &[0u8; 32]),
        &(env.ledger().timestamp() + 86_400),
    );
    assert_eq!(result, Err(Ok(ContractError::FeatureDisabled)));
}

// ── execute_dca_interval blocked when flag disabled ───────────────────────────

#[test]
fn execute_dca_interval_blocked_when_flag_disabled() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);

    client.set_feature_flag(&String::from_str(&env, FEAT_DCA), &false);

    let user = Address::generate(&env);

    let result = client.try_execute_dca_interval(&user, &1u64);
    assert_eq!(result, Err(Ok(ContractError::FeatureDisabled)));
}
