//! Authorization conformance tests for `bridge` admin entrypoints (Issue #1335).
//!
//! `bridge`'s privileged entrypoints guard `admin.require_auth()` behind
//! `if !cfg!(test) { admin.require_auth(); }` (see
//! `contracts/bridge/src/lib.rs`, e.g. `register_wrapped_asset`,
//! `set_reserve_threshold`, `set_governance`). That means the auth check is
//! compiled out entirely whenever `bridge`'s own crate is built in test
//! mode (`cargo test -p bridge`) -- so a unit test living inside the
//! `bridge` crate itself can never actually observe `require_auth` being
//! enforced, no matter how the mocked auths are set up.
//!
//! This crate pulls in `bridge` as an ordinary (non-test-mode) dependency,
//! so `cfg!(test)` evaluates to `false` for the `bridge` crate here and the
//! real `require_auth()` calls run. That makes `integration_tests` the
//! correct place for the conformance policy this issue asks for: the
//! recorded expectation is "every one of these admin-gated entrypoints
//! must reject a call with no authorizing signature," and clearing mocked
//! auths (`env.set_auths(&[])`) and calling the real (non-`try_`) admin
//! address proves it actually checks the signature, not just storage
//! equality (`require_admin` only compares the stored admin address to the
//! caller-supplied one; it does not by itself prove a signature was
//! required).

use bridge::{BridgeContract, BridgeContractClient, ChainId};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

fn setup() -> (Env, Address, BridgeContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(BridgeContract, ());
    let client = BridgeContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let validators = Vec::from_array(&env, [Address::generate(&env)]);
    client.initialize(&admin, &validators, &1u32, &1_000_000_000i128, &1_000_000_000i128, &0u64);
    (env, admin, client)
}

/// Expected authorization policy for every admin-gated entrypoint under
/// test: it must require the stored admin's real signature, not merely
/// the admin's address being passed as an argument. Entries that are read
/// or state-mutating-but-permissionless by design are intentionally
/// excluded (see `get_reserve_threshold`, `get_governance`, etc., which
/// take no admin argument at all).
#[test]
fn register_wrapped_asset_requires_admin_auth() {
    let (env, admin, client) = setup();
    env.set_auths(&[]);
    let result = client.try_register_wrapped_asset(
        &admin,
        &ChainId::Ethereum,
        &String::from_str(&env, "USDC"),
        &String::from_str(&env, "wUSDC"),
        &6u32,
    );
    assert!(
        result.is_err(),
        "register_wrapped_asset must reject a call with no authorizing signature, \
         even though the correct admin address was supplied"
    );
}

#[test]
fn set_reserve_threshold_requires_admin_auth() {
    let (env, admin, client) = setup();
    env.set_auths(&[]);
    let result = client.try_set_reserve_threshold(&admin, &5_000u32);
    assert!(
        result.is_err(),
        "set_reserve_threshold must reject a call with no authorizing signature"
    );
}

#[test]
fn set_governance_requires_admin_auth() {
    let (env, admin, client) = setup();
    let governance = Address::generate(&env);
    env.set_auths(&[]);
    let result = client.try_set_governance(&admin, &governance);
    assert!(
        result.is_err(),
        "set_governance must reject a call with no authorizing signature"
    );
}

#[test]
fn set_deployment_id_requires_admin_auth() {
    let (env, admin, client) = setup();
    let new_deployment = Address::generate(&env);
    env.set_auths(&[]);
    let result = client.try_set_deployment_id(&admin, &new_deployment);
    assert!(
        result.is_err(),
        "set_deployment_id must reject a call with no authorizing signature"
    );
}

/// Negative control: proves the harness above would actually catch a
/// regression. With auths intact (not cleared), the same admin-gated call
/// must succeed -- confirming `try_set_reserve_threshold`'s failure in
/// `set_reserve_threshold_requires_admin_auth` is caused by the missing
/// signature, not by some unrelated setup error.
#[test]
fn set_reserve_threshold_succeeds_with_real_admin_auth() {
    let (_env, admin, client) = setup();
    let result = client.try_set_reserve_threshold(&admin, &5_000u32);
    assert!(
        result.is_ok(),
        "set_reserve_threshold must succeed for the real admin with a valid signature: {result:?}"
    );
}
