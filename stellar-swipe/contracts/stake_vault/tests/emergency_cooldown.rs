#![cfg(test)]

//! Integration tests for issue #1026: emergency withdrawal cooldown policy.
//!
//! These live in their own integration-test binary (rather than the in-crate
//! `tests` module) so the cooldown behaviour is exercised end-to-end through the
//! public contract client.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};
use stake_vault::{StakeVaultContract, StakeVaultContractClient, StakeVaultError};

struct Ctx<'a> {
    env: Env,
    client: StakeVaultContractClient<'a>,
    admin: Address,
    token: Address,
    signer: Address,
}

fn setup<'a>() -> Ctx<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let registry = Address::generate(&env);
    let signer = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let id = env.register(StakeVaultContract, ());
    let client = StakeVaultContractClient::new(&env, &id);
    client.initialize(&admin, &token, &registry);

    // 1-of-1 multi-sig, no penalty, no request timeout.
    let admins: Vec<Address> = vec![&env, signer.clone()];
    client.configure_emergency_multisig(&admin, &admins, &1u32, &0u32, &0u64);

    Ctx {
        env,
        client,
        admin,
        token,
        signer,
    }
}

fn stake(ctx: &Ctx, staker: &Address, amount: i128) {
    StellarAssetClient::new(&ctx.env, &ctx.token).mint(staker, &amount);
    ctx.client.deposit_stake(staker, &amount);
}

/// Drives one full emergency unstake (request + single approval) to completion.
fn emergency_unstake(ctx: &Ctx, staker: &Address) {
    ctx.client.request_emergency_unstake(staker);
    ctx.client.approve_emergency_unstake(&ctx.signer, staker);
    assert!(ctx.client.get_emergency_request(staker).is_none());
}

#[test]
fn second_request_within_cooldown_is_rejected_with_precise_error() {
    let ctx = setup();
    ctx.client.set_emergency_cooldown(&ctx.admin, &3_600u64);

    let staker = Address::generate(&ctx.env);
    ctx.env.ledger().with_mut(|l| l.timestamp = 10_000);
    stake(&ctx, &staker, 1_000_000);
    emergency_unstake(&ctx, &staker);

    // Re-stake and immediately try again — still inside the cooldown window.
    stake(&ctx, &staker, 1_000_000);
    let err = ctx
        .client
        .try_request_emergency_unstake(&staker)
        .err()
        .unwrap()
        .unwrap();
    assert_eq!(err, StakeVaultError::EmergencyCooldownActive);
}

#[test]
fn remaining_cooldown_is_queryable_and_counts_down() {
    let ctx = setup();
    ctx.client.set_emergency_cooldown(&ctx.admin, &3_600u64);

    let staker = Address::generate(&ctx.env);
    ctx.env.ledger().with_mut(|l| l.timestamp = 10_000);
    stake(&ctx, &staker, 1_000_000);

    // No completed emergency unstake yet → no cooldown.
    assert_eq!(ctx.client.emergency_cooldown_remaining(&staker), 0);

    emergency_unstake(&ctx, &staker);
    assert_eq!(ctx.client.emergency_cooldown_remaining(&staker), 3_600);

    ctx.env.ledger().with_mut(|l| l.timestamp = 10_000 + 1_000);
    assert_eq!(ctx.client.emergency_cooldown_remaining(&staker), 2_600);
}

#[test]
fn request_succeeds_once_the_cooldown_window_has_elapsed() {
    let ctx = setup();
    ctx.client.set_emergency_cooldown(&ctx.admin, &3_600u64);

    let staker = Address::generate(&ctx.env);
    ctx.env.ledger().with_mut(|l| l.timestamp = 10_000);
    stake(&ctx, &staker, 1_000_000);
    emergency_unstake(&ctx, &staker);

    stake(&ctx, &staker, 1_000_000);
    ctx.env
        .ledger()
        .with_mut(|l| l.timestamp = 10_000 + 3_600 + 1);
    assert_eq!(ctx.client.emergency_cooldown_remaining(&staker), 0);

    // Now the request is accepted again.
    ctx.client.request_emergency_unstake(&staker);
    assert!(ctx.client.get_emergency_request(&staker).is_some());
}

#[test]
fn cooldown_defaults_to_disabled() {
    let ctx = setup();
    // No set_emergency_cooldown call.

    let staker = Address::generate(&ctx.env);
    ctx.env.ledger().with_mut(|l| l.timestamp = 10_000);
    stake(&ctx, &staker, 1_000_000);
    emergency_unstake(&ctx, &staker);

    assert_eq!(ctx.client.emergency_cooldown_remaining(&staker), 0);

    // Back-to-back emergency unstakes are allowed when cooldown is unset.
    stake(&ctx, &staker, 1_000_000);
    ctx.client.request_emergency_unstake(&staker);
    assert!(ctx.client.get_emergency_request(&staker).is_some());
}

#[test]
fn only_admin_can_set_the_cooldown() {
    let ctx = setup();
    let intruder = Address::generate(&ctx.env);

    let err = ctx
        .client
        .try_set_emergency_cooldown(&intruder, &3_600u64)
        .err()
        .unwrap()
        .unwrap();
    assert_eq!(err, StakeVaultError::Unauthorized);
}

#[test]
fn cooldown_is_tracked_per_account() {
    let ctx = setup();
    ctx.client.set_emergency_cooldown(&ctx.admin, &3_600u64);

    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);
    ctx.env.ledger().with_mut(|l| l.timestamp = 10_000);

    stake(&ctx, &alice, 1_000_000);
    emergency_unstake(&ctx, &alice);

    // Bob has never done an emergency unstake — he is not affected by Alice's.
    stake(&ctx, &bob, 1_000_000);
    assert_eq!(ctx.client.emergency_cooldown_remaining(&bob), 0);
    ctx.client.request_emergency_unstake(&bob);
    assert!(ctx.client.get_emergency_request(&bob).is_some());
}
