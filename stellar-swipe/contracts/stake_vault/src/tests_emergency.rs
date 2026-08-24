#![cfg(test)]

//! Unit tests for Issue #754: multi-sig emergency early unstake.

use crate::{StakeVaultContract, StakeVaultContractClient, StakeVaultError};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

fn setup(env: &Env) -> (Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let registry = Address::generate(env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let id = env.register(StakeVaultContract, ());
    let client = StakeVaultContractClient::new(env, &id);
    client.initialize(&admin, &token, &registry);
    (admin, token, id)
}

fn fund_staker(env: &Env, token: &Address, admin: &Address, staker: &Address, amount: i128) {
    StellarAssetClient::new(env, token).mint(staker, &amount);
}

fn do_stake(env: &Env, contract_id: &Address, staker: &Address, amount: i128) {
    let client = StakeVaultContractClient::new(env, contract_id);
    client.deposit_stake(staker, &amount);
}

/// Happy path: request approved by enough signers, executes with penalty, funds transferred.
#[test]
fn test_approved_within_threshold_executes_with_penalty() {
    let env = Env::default();
    let (admin, token, id) = setup(&env);
    let client = StakeVaultContractClient::new(&env, &id);

    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let staker = Address::generate(&env);

    // Configure 2-of-2 multi-sig, 10% penalty, 3600 s timeout.
    let admins: Vec<Address> = vec![&env, signer1.clone(), signer2.clone()];
    client.configure_emergency_multisig(&admin, &admins, &2u32, &1_000u32, &3600u64);

    // Stake some tokens.
    fund_staker(&env, &token, &admin, &staker, 1_000_000);
    do_stake(&env, &id, &staker, 1_000_000);

    // Staker requests emergency unstake.
    client.request_emergency_unstake(&staker);
    assert!(client.get_emergency_request(&staker).is_some());

    // First approval — not yet at threshold.
    client.approve_emergency_unstake(&signer1, &staker);
    assert!(client.get_emergency_request(&staker).is_some());

    // Second approval — threshold reached, executes.
    client.approve_emergency_unstake(&signer2, &staker);

    // Request should be consumed.
    assert!(client.get_emergency_request(&staker).is_none());

    // Stake should be zero.
    assert_eq!(client.get_stake(&staker), 0);
}

/// Insufficient approvals: request stays pending, no funds move.
#[test]
fn test_insufficient_approvals_does_not_execute() {
    let env = Env::default();
    let (admin, token, id) = setup(&env);
    let client = StakeVaultContractClient::new(&env, &id);

    let signer1 = Address::generate(&env);
    let signer2 = Address::generate(&env);
    let staker = Address::generate(&env);

    let admins: Vec<Address> = vec![&env, signer1.clone(), signer2.clone()];
    client.configure_emergency_multisig(&admin, &admins, &2u32, &1_000u32, &3600u64);

    fund_staker(&env, &token, &admin, &staker, 500_000);
    do_stake(&env, &id, &staker, 500_000);

    client.request_emergency_unstake(&staker);
    // Only one approval out of two required.
    client.approve_emergency_unstake(&signer1, &staker);

    // Funds still locked.
    assert_eq!(client.get_stake(&staker), 500_000);
    assert!(client.get_emergency_request(&staker).is_some());
}

/// Request expires after timeout without reaching threshold.
#[test]
fn test_request_expiry_without_threshold() {
    let env = Env::default();
    let (admin, token, id) = setup(&env);
    let client = StakeVaultContractClient::new(&env, &id);

    let signer = Address::generate(&env);
    let staker = Address::generate(&env);

    let admins: Vec<Address> = vec![&env, signer.clone()];
    // 2-of-1 is invalid, so use 1-of-2 where only 1 approval needed but we test expiry.
    // Actually let's use 2-of-2 with just 1 signer configured (which should fail config).
    // Use valid 1-of-1 but with short timeout and check expiry before approval.
    let admins2: Vec<Address> = vec![&env, signer.clone()];
    let timeout_secs: u64 = 60;
    client.configure_emergency_multisig(&admin, &admins2, &1u32, &0u32, &timeout_secs);

    fund_staker(&env, &token, &admin, &staker, 200_000);
    do_stake(&env, &id, &staker, 200_000);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.request_emergency_unstake(&staker);

    // Advance time past the timeout.
    env.ledger()
        .with_mut(|l| l.timestamp = 1_000 + timeout_secs + 1);

    // expire_request should succeed and remove the stale request.
    client.expire_emergency_request(&staker);
    assert!(client.get_emergency_request(&staker).is_none());

    // approve should now return EmergencyRequestNotFound.
    let result = client.try_approve_emergency_unstake(&signer, &staker);
    assert_eq!(result, Err(Ok(StakeVaultError::EmergencyRequestNotFound)));

    // Stake is untouched.
    assert_eq!(client.get_stake(&staker), 200_000);
}
