#![cfg(test)]

extern crate std;

use crate::categories::{RiskLevel, SignalCategory};
use crate::errors::{AdminError, SignalCancelError};
use crate::types::{SignalAction, SignalStatus};
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

fn setup() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(10_000);
    #[allow(deprecated)]
    let id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, admin, client)
}

fn create_signal(
    env: &Env,
    client: &SignalRegistryClient,
    provider: &Address,
    expiry_offset: u64,
) -> u64 {
    let expiry = env.ledger().timestamp() + expiry_offset;
    client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "Active cap test rationale"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(env),
        &RiskLevel::Medium,
    )
}

fn try_create_signal(
    env: &Env,
    client: &SignalRegistryClient,
    provider: &Address,
) -> Result<Result<u64, soroban_sdk::Error>, Result<AdminError, soroban_sdk::InvokeError>> {
    let expiry = env.ledger().timestamp() + 86_400;
    client.try_create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "Active cap test rationale"),
        &expiry,
        &SignalCategory::SWING,
        &Vec::new(env),
        &RiskLevel::Medium,
    )
}

#[test]
fn admin_configures_tier_active_signal_caps() {
    let (_, admin, client) = setup();

    client.set_tier_signal_limits(&admin, &2u32, &4u32, &8u32);
    let config = client.get_config();

    assert_eq!(config.bronze_signal_limit, 2);
    assert_eq!(config.silver_signal_limit, 4);
    assert_eq!(config.gold_signal_limit, 8);
}

#[test]
fn provider_can_submit_until_active_cap_then_rejected() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    client.set_tier_signal_limits(&admin, &2u32, &2u32, &2u32);

    create_signal(&env, &client, &provider, 86_400);
    create_signal(&env, &client, &provider, 86_400);

    let result = try_create_signal(&env, &client, &provider);
    assert_eq!(result, Err(Ok(AdminError::SignalLimitExceeded)));
}

#[test]
fn cleanup_expiry_releases_provider_capacity() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    client.set_tier_signal_limits(&admin, &1u32, &1u32, &1u32);

    let id = create_signal(&env, &client, &provider, 10);
    assert_eq!(
        try_create_signal(&env, &client, &provider),
        Err(Ok(AdminError::SignalLimitExceeded))
    );

    env.ledger().set_timestamp(env.ledger().timestamp() + 11);
    let (_, expired) = client.cleanup_expired_signals(&10);
    assert_eq!(expired, 1);
    assert_eq!(
        client.get_signal(&id).unwrap().status,
        SignalStatus::Expired
    );

    create_signal(&env, &client, &provider, 86_400);
}

#[test]
fn lazy_get_expiry_releases_provider_capacity() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    client.set_tier_signal_limits(&admin, &1u32, &1u32, &1u32);

    let id = create_signal(&env, &client, &provider, 10);
    env.ledger().set_timestamp(env.ledger().timestamp() + 11);

    assert_eq!(
        client.get_signal(&id).unwrap().status,
        SignalStatus::Expired
    );
    create_signal(&env, &client, &provider, 86_400);
}

#[test]
fn provider_cancellation_releases_capacity() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    client.set_tier_signal_limits(&admin, &1u32, &1u32, &1u32);

    let id = create_signal(&env, &client, &provider, 86_400);
    assert_eq!(
        try_create_signal(&env, &client, &provider),
        Err(Ok(AdminError::SignalLimitExceeded))
    );

    client.cancel_signal(&provider, &id);
    create_signal(&env, &client, &provider, 86_400);
}

#[test]
fn resolved_signal_releases_capacity() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);
    client.set_tier_signal_limits(&admin, &1u32, &1u32, &1u32);

    let id = create_signal(&env, &client, &provider, 86_400);
    client.record_trade_execution(&executor, &id, &100i128, &110i128, &1_000i128);

    assert_eq!(
        client.get_signal(&id).unwrap().status,
        SignalStatus::Successful
    );
    create_signal(&env, &client, &provider, 86_400);
}

#[test]
fn rejected_cancellation_does_not_release_capacity() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    client.set_tier_signal_limits(&admin, &1u32, &1u32, &1u32);
    client.set_min_signal_lifetime(&admin, &3_600u64);

    let id = create_signal(&env, &client, &provider, 86_400);
    let result = client.try_cancel_signal(&provider, &id);

    assert_eq!(result, Err(Ok(SignalCancelError::LifetimeNotElapsed)));
    assert_eq!(
        try_create_signal(&env, &client, &provider),
        Err(Ok(AdminError::SignalLimitExceeded))
    );
}

#[test]
fn build_info_exposes_source_hash_and_git_commit() {
    let (env, _, client) = setup();
    let info = client.get_build_info();

    assert_eq!(
        info.get(String::from_str(&env, "version")).unwrap(),
        String::from_str(&env, env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        info.get(String::from_str(&env, "source_hash")).unwrap(),
        String::from_str(&env, env!("STELLAR_SOURCE_HASH"))
    );
    assert_eq!(
        info.get(String::from_str(&env, "git_commit")).unwrap(),
        String::from_str(&env, env!("STELLAR_GIT_COMMIT"))
    );
}
