#![cfg(test)]
//! Security regression tests for:
//! - Issue #811 — upgrade-safe contract versioning (shared::version wired
//!   into initialize()/upgrade()).
//! - Issue #813 — cross-contract authentication hardening for fee collector
//!   operations (authorized-caller allowlist).
//!
//! # Note on testing `upgrade()`
//! `upgrade()` calls `Deployer::update_current_contract_wasm`, which requires
//! a real Wasm blob previously uploaded via `Deployer::upload_contract_wasm`.
//! No such blob is available inside a `cargo test` unit-test run (the crate
//! under test is not itself compiled to Wasm as part of `cargo test`), so —
//! consistent with `contracts/integration_tests/.../test_contract_upgrade.rs`,
//! which documents the same limitation — these tests cover every guard that
//! runs *before* the Wasm swap (auth, version compatibility) rather than the
//! swap itself.

use soroban_sdk::{testutils::Address as _, Address, BytesN, Env};

use crate::{ContractError, FeeCollector, FeeCollectorClient};

fn setup(env: &Env) -> (Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);
    (admin, client)
}

fn dummy_wasm_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[7u8; 32])
}

// ── Issue #811: contract versioning ─────────────────────────────────────────

#[test]
fn initialize_sets_contract_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);

    assert_eq!(client.get_contract_version(), 2);
}

#[test]
fn upgrade_rejects_same_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client) = setup(&env);
    let _ = admin;

    let current = client.get_contract_version();
    let result = client.try_upgrade(&dummy_wasm_hash(&env), &current);
    assert_eq!(
        result,
        Err(Ok(ContractError::IncompatibleContractVersion))
    );
    // Version must be unchanged — the guard rejected before any state mutation.
    assert_eq!(client.get_contract_version(), current);
}

#[test]
fn upgrade_rejects_downgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);

    let current = client.get_contract_version();
    let result = client.try_upgrade(&dummy_wasm_hash(&env), &(current - 1));
    assert_eq!(
        result,
        Err(Ok(ContractError::IncompatibleContractVersion))
    );
}

#[test]
fn upgrade_requires_admin_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let current = client.get_contract_version();

    // Clear mocked auths so require_auth() is actually enforced, then invoke
    // with no authorizing signature at all.
    env.set_auths(&[]);
    let result = client.try_upgrade(&dummy_wasm_hash(&env), &(current + 1));
    assert!(result.is_err(), "upgrade without admin auth must be rejected");
}

// ── Issue #813: authorized-caller allowlist ─────────────────────────────────

#[test]
fn caller_not_authorized_by_default() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let stranger = Address::generate(&env);

    assert!(!client.is_caller_authorized(&stranger));
}

#[test]
fn admin_can_authorize_and_revoke_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let keeper = Address::generate(&env);

    client.authorize_caller(&keeper);
    assert!(client.is_caller_authorized(&keeper));

    client.revoke_caller(&keeper);
    assert!(!client.is_caller_authorized(&keeper));
}

#[test]
fn record_provider_fee_share_rejects_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let stranger = Address::generate(&env);

    let result = client.try_record_provider_fee_share(&stranger, &provider, &1_000i128);
    assert_eq!(result, Err(Ok(ContractError::UnauthorizedCaller)));
}

#[test]
fn record_provider_fee_share_allows_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client) = setup(&env);
    let provider = Address::generate(&env);

    client.record_provider_fee_share(&admin, &provider, &1_000i128);
}

#[test]
fn record_provider_fee_share_allows_authorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let keeper = Address::generate(&env);

    client.authorize_caller(&keeper);
    client.record_provider_fee_share(&keeper, &provider, &1_000i128);
}

#[test]
fn record_provider_fee_share_rejects_after_revocation() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let keeper = Address::generate(&env);

    client.authorize_caller(&keeper);
    client.record_provider_fee_share(&keeper, &provider, &1_000i128);

    client.revoke_caller(&keeper);
    let result = client.try_record_provider_fee_share(&keeper, &provider, &1_000i128);
    assert_eq!(result, Err(Ok(ContractError::UnauthorizedCaller)));
}

#[test]
fn set_congestion_signal_rejects_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let stranger = Address::generate(&env);

    let result = client.try_set_congestion_signal(&stranger, &12_000u32);
    assert_eq!(result, Err(Ok(ContractError::UnauthorizedCaller)));
}

#[test]
fn set_congestion_signal_allows_authorized_keeper() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let keeper = Address::generate(&env);

    client.authorize_caller(&keeper);
    client.set_congestion_signal(&keeper, &12_000u32);
}

#[test]
fn only_admin_can_authorize_callers() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let keeper = Address::generate(&env);

    // Clear mocked auths: authorize_caller requires the *stored admin's*
    // signature, not the caller's own — a non-admin invoking it must fail.
    env.set_auths(&[]);
    let result = client.try_authorize_caller(&keeper);
    assert!(result.is_err());
}
