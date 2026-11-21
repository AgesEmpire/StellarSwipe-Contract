#![cfg(test)]
//! Issue #1217: bridge messages must not execute after their validity window.
//!
//! A message is valid while `ledger timestamp < expires_at`; at and after
//! `expires_at` it can neither be approved nor executed, and a rejected
//! execution leaves balances, supply and transfer status untouched.

use crate::{
    BridgeContract, BridgeContractClient, BridgeError, ChainId, TransferStatus,
    DEFAULT_MESSAGE_VALIDITY_SECONDS, MAX_MESSAGE_VALIDITY_SECONDS,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};

const START: u64 = 1_000;
const EXPIRES_AT: u64 = START + 100;
const AMOUNT: i128 = 500;

struct Setup {
    env: Env,
    client: BridgeContractClient<'static>,
    admin: Address,
    validators: Vec<Address>,
    user: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);
    let id = env.register(BridgeContract, ());
    let client = BridgeContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let mut validators = Vec::new(&env);
    for _ in 0..3 {
        validators.push_back(Address::generate(&env));
    }
    client.initialize(&admin, &validators, &2, &1_000, &1_000, &600);
    client.register_wrapped_asset(
        &admin,
        &ChainId::Ethereum,
        &String::from_str(&env, "ETH"),
        &String::from_str(&env, "wETH"),
        &18,
    );
    let user = Address::generate(&env);
    Setup {
        env,
        client,
        admin,
        validators,
        user,
    }
}

fn initiate(s: &Setup, nonce: u64, expires_at: u64) -> Result<u64, BridgeError> {
    match s.client.try_initiate_lock_mint_with_expiry(
        &s.user,
        &ChainId::Ethereum,
        &ChainId::Polygon,
        &String::from_str(&s.env, "ETH"),
        &String::from_str(&s.env, "wETH"),
        &AMOUNT,
        &String::from_str(&s.env, "0xexpiry"),
        &nonce,
        &String::from_str(&s.env, "stellar:user"),
        &expires_at,
    ) {
        Ok(Ok(id)) => Ok(id),
        Err(Ok(e)) => Err(e),
        other => panic!("unexpected result: {:?}", other),
    }
}

fn approve(s: &Setup, transfer_id: u64, idx: u32) -> Result<(), BridgeError> {
    match s.client.try_approve_lock_mint(
        &s.validators.get(idx).unwrap(),
        &transfer_id,
        &String::from_str(&s.env, if idx == 0 { "sig-a" } else { "sig-b" }),
    ) {
        Ok(Ok(())) => Ok(()),
        Err(Ok(e)) => Err(e),
        other => panic!("unexpected result: {:?}", other),
    }
}

fn execute(s: &Setup, transfer_id: u64) -> Result<(), BridgeError> {
    match s.client.try_execute_lock_mint(&s.admin, &transfer_id) {
        Ok(Ok(())) => Ok(()),
        Err(Ok(e)) => Err(e),
        other => panic!("unexpected result: {:?}", other),
    }
}

/// Registers a message expiring at `EXPIRES_AT` and brings it to quorum.
fn ready_transfer(s: &Setup) -> u64 {
    let id = initiate(s, 1, EXPIRES_AT).unwrap();
    approve(s, id, 0).unwrap();
    approve(s, id, 1).unwrap();
    assert_eq!(
        s.client.get_transfer(&id).status,
        TransferStatus::ReadyToExecute
    );
    id
}

fn wrapped_balance(s: &Setup) -> i128 {
    s.client
        .get_wrapped_balance(&s.user, &String::from_str(&s.env, "wETH"))
}

fn assert_nothing_minted(s: &Setup, id: u64) {
    assert_eq!(wrapped_balance(s), 0);
    assert_eq!(s.client.get_total_minted(), 0);
    let t = s.client.get_transfer(&id);
    assert_eq!(t.status, TransferStatus::ReadyToExecute);
    assert_eq!(t.executed_at, None);
}

// ── Execution boundary ───────────────────────────────────────────────────────

#[test]
fn execute_just_before_expiry_succeeds() {
    let s = setup();
    let id = ready_transfer(&s);

    s.env.ledger().set_timestamp(EXPIRES_AT - 1);
    assert_eq!(execute(&s, id), Ok(()));
    assert_eq!(wrapped_balance(&s), AMOUNT);
    assert_eq!(s.client.get_total_minted(), AMOUNT);
    assert_eq!(s.client.get_transfer(&id).status, TransferStatus::Completed);
}

#[test]
fn execute_at_exact_expiry_is_rejected_without_state_change() {
    let s = setup();
    let id = ready_transfer(&s);

    s.env.ledger().set_timestamp(EXPIRES_AT);
    assert_eq!(execute(&s, id), Err(BridgeError::MessageExpired));
    assert_nothing_minted(&s, id);
}

#[test]
fn execute_just_after_expiry_is_rejected_without_state_change() {
    let s = setup();
    let id = ready_transfer(&s);

    s.env.ledger().set_timestamp(EXPIRES_AT + 1);
    assert_eq!(execute(&s, id), Err(BridgeError::MessageExpired));
    assert_nothing_minted(&s, id);

    // Repeated attempts keep failing; nothing was consumed or minted.
    s.env
        .ledger()
        .set_timestamp(EXPIRES_AT + DEFAULT_MESSAGE_VALIDITY_SECONDS);
    assert_eq!(execute(&s, id), Err(BridgeError::MessageExpired));
    assert_nothing_minted(&s, id);
}

#[test]
fn expired_message_rejected_even_after_retry_reset() {
    let s = setup();
    let id = ready_transfer(&s);

    s.client
        .mark_transfer_failed(&s.admin, &id, &String::from_str(&s.env, "rpc"), &true);
    s.client.retry_transfer(&s.admin, &id);

    s.env.ledger().set_timestamp(EXPIRES_AT);
    assert_eq!(execute(&s, id), Err(BridgeError::MessageExpired));
    assert_nothing_minted(&s, id);
}

// ── Approval boundary ────────────────────────────────────────────────────────

#[test]
fn approval_boundary_follows_expiry() {
    let s = setup();
    let id = initiate(&s, 1, EXPIRES_AT).unwrap();

    s.env.ledger().set_timestamp(EXPIRES_AT - 1);
    assert_eq!(approve(&s, id, 0), Ok(()));

    s.env.ledger().set_timestamp(EXPIRES_AT);
    assert_eq!(approve(&s, id, 1), Err(BridgeError::MessageExpired));
    let t = s.client.get_transfer(&id);
    assert_eq!(t.approvals.len(), 1);
    assert_eq!(t.status, TransferStatus::PendingValidators);
}

// ── Registration ─────────────────────────────────────────────────────────────

#[test]
fn initiate_rejects_expiry_at_or_before_now() {
    let s = setup();
    assert_eq!(initiate(&s, 1, START), Err(BridgeError::MessageExpired));
    assert_eq!(initiate(&s, 1, START - 1), Err(BridgeError::MessageExpired));

    // The rejected attempts wrote no replay lock: the same message with a
    // valid expiry is still accepted.
    let id = initiate(&s, 1, START + 1).unwrap();
    assert_eq!(s.client.get_transfer(&id).expires_at, START + 1);
}

#[test]
fn initiate_bounds_expiry_to_max_validity() {
    let s = setup();
    assert_eq!(
        initiate(&s, 1, START + MAX_MESSAGE_VALIDITY_SECONDS + 1),
        Err(BridgeError::InvalidMessageExpiry)
    );
    let id = initiate(&s, 1, START + MAX_MESSAGE_VALIDITY_SECONDS).unwrap();
    assert_eq!(
        s.client.get_transfer(&id).expires_at,
        START + MAX_MESSAGE_VALIDITY_SECONDS
    );
}

#[test]
fn legacy_initiate_applies_default_validity_window() {
    let s = setup();
    let id = s.client.initiate_lock_mint(
        &s.user,
        &ChainId::Ethereum,
        &ChainId::Polygon,
        &String::from_str(&s.env, "ETH"),
        &String::from_str(&s.env, "wETH"),
        &AMOUNT,
        &String::from_str(&s.env, "0xlegacy"),
        &1,
        &String::from_str(&s.env, "stellar:user"),
    );
    assert_eq!(
        s.client.get_transfer(&id).expires_at,
        START + DEFAULT_MESSAGE_VALIDITY_SECONDS
    );
}

// ── Signature binding ────────────────────────────────────────────────────────

#[test]
fn digest_binds_expiry() {
    let s = setup();
    let a = initiate(&s, 1, EXPIRES_AT).unwrap();
    let digest_a = s.client.get_transfer_digest(&a);

    // Same message fields except the expiry → different signed digest.
    let contract_id = s.client.address.clone();
    let tampered = s.env.as_contract(&contract_id, || {
        let mut t = BridgeContract::get_transfer(s.env.clone(), a).unwrap();
        t.expires_at = EXPIRES_AT + 1;
        crate::transfer_message_digest(&s.env, &t)
    });
    assert_ne!(digest_a, tampered);
    // Digest is stable for an unchanged message.
    assert_eq!(digest_a, s.client.get_transfer_digest(&a));
}
