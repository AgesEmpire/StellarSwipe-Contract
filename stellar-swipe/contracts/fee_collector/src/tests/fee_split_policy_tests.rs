#![cfg(test)]

//! Contract-level coverage for the configurable fee split policy (Issue #1032).

use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env,
};

use crate::{ContractError, FeeCollector, FeeCollectorClient, FeeSplitPolicy};

fn setup(env: &Env) -> (Address, Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);
    (admin, contract_id, client)
}

#[test]
fn default_policy_is_30_70() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let policy = client.get_fee_split_policy();
    assert_eq!(
        policy,
        FeeSplitPolicy {
            protocol_bps: 3_000,
            provider_bps: 7_000,
        }
    );
}

#[test]
fn admin_can_update_policy_and_event_is_emitted() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    client.set_fee_split_policy(&2_500, &7_500);
    // The update is auditable via an emitted event (checked on the invocation
    // that produced it, before any further calls reset the event buffer).
    assert!(!env.events().all().is_empty());

    assert_eq!(
        client.get_fee_split_policy(),
        FeeSplitPolicy {
            protocol_bps: 2_500,
            provider_bps: 7_500,
        }
    );
}

#[test]
fn policy_that_does_not_sum_to_100pct_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let res = client.try_set_fee_split_policy(&4_000, &4_000);
    assert_eq!(res, Err(Ok(ContractError::InvalidFeeConfiguration)));

    // State is unchanged after a rejected update.
    assert_eq!(
        client.get_fee_split_policy(),
        FeeSplitPolicy {
            protocol_bps: 3_000,
            provider_bps: 7_000,
        }
    );
}

#[test]
fn preview_split_uses_active_policy() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    assert_eq!(client.preview_fee_split(&1_000), (300, 700));

    client.set_fee_split_policy(&1_000, &9_000);
    assert_eq!(client.preview_fee_split(&1_000), (100, 900));

    // Dust-free: shares always reconstruct the gross amount.
    let (p, q) = client.preview_fee_split(&777);
    assert_eq!(p + q, 777);
}

#[test]
fn record_from_gross_credits_provider_and_returns_protocol_share() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);
    let provider = Address::generate(&env);

    client.set_fee_split_policy(&2_000, &8_000);
    let (protocol_amount, provider_amount) =
        client.record_provider_gross_fee_share(&admin, &provider, &10_000);

    assert_eq!(protocol_amount, 2_000);
    assert_eq!(provider_amount, 8_000);

    let report = client.get_provider_earnings_report(&provider, &crate::ReportPeriod::AllTime);
    assert_eq!(report.fee_shares_earned, 8_000);
}

#[test]
fn record_from_gross_rejects_non_positive_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);
    let provider = Address::generate(&env);

    let res = client.try_record_provider_gross_fee_share(&admin, &provider, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
}

#[test]
fn record_from_gross_rejects_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);
    let stranger = Address::generate(&env);
    let provider = Address::generate(&env);

    let res = client.try_record_provider_gross_fee_share(&stranger, &provider, &1_000);
    assert_eq!(res, Err(Ok(ContractError::UnauthorizedCaller)));
}
