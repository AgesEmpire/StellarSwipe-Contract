#![cfg(test)]
//! Integration tests for the nonce replay-protection audit on `execute_copy_trade`
//! (Issue: replay attack prevention audit for execute_copy_trade nonce handling).
//!
//! Covers the three scenarios from the audit's acceptance criteria:
//! (a) the same nonce submitted twice in the same ledger is rejected with
//!     `ContractError::NonceAlreadyUsed`,
//! (b) a nonce submitted after `expiry_ts` is rejected with
//!     `ContractError::TradeExpired`,
//! (c) a committed nonce cannot be replayed across a simulated contract
//!     upgrade / storage migration (ledger sequence jump), since nonces live
//!     in persistent storage rather than instance/temporary storage.
//!
//! Also covers `purge_expired_nonces`, the admin/keeper maintenance entrypoint
//! that reclaims persistent storage for expired replay-protection entries.

use crate::{errors::ContractError, OrderType, TradeExecutorContract, TradeExecutorContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
    Address, Bytes, Env,
};

// ── Mock UserPortfolio (mirrors `crate::test`'s fixture; kept self-contained
// so this file has no dependency on the private `test` module) ────────────────

#[soroban_sdk::contract]
pub struct MockPortfolio;

#[soroban_sdk::contracttype]
#[derive(Clone)]
enum PortfolioKey {
    Count(Address),
}

#[soroban_sdk::contractimpl]
impl MockPortfolio {
    pub fn validate_and_record(env: Env, user: Address, max_positions: u32) -> u32 {
        let key = PortfolioKey::Count(user.clone());
        let count: u32 = env.storage().instance().get(&key).unwrap_or(0);
        if count >= max_positions {
            panic!("position limit reached");
        }
        let new_count = count + 1;
        env.storage().instance().set(&key, &new_count);
        new_count
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const TRADE_AMOUNT: i128 = 1_000_000;

fn sac(env: &Env) -> Address {
    let issuer = Address::generate(env);
    env.register_stellar_asset_contract_v2(issuer).address()
}

fn test_tx_hash(env: &Env, seed: u8) -> Bytes {
    let mut arr = [0u8; 32];
    arr[0] = seed;
    arr[31] = seed;
    Bytes::from_array(env, &arr)
}

fn far_future(env: &Env) -> u64 {
    env.ledger().timestamp() + 86_400 * 365
}

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac(&env);
    let portfolio_id = env.register(MockPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    StellarAssetClient::new(&env, &token).mint(&user, &(TRADE_AMOUNT * 100));

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    (env, exec_id, user, token)
}

// ── (a) Same nonce, same ledger ────────────────────────────────────────────────

#[test]
fn same_nonce_resubmitted_same_ledger_is_rejected() {
    let (env, exec_id, user, token) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 1),
        &far_future(&env),
    );

    // Same nonce, same ledger, different tx_hash — still rejected because the
    // nonce counter has already advanced past 1.
    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 2),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::NonceAlreadyUsed)));
}

// ── (b) Nonce submitted after expiry ───────────────────────────────────────────

#[test]
fn nonce_submitted_after_expiry_is_rejected() {
    let (env, exec_id, user, token) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().set_timestamp(1_000);

    // expiry_ts (999) is already in the past relative to `now` (1_000).
    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 1),
        &999u64,
    );
    assert_eq!(err, Err(Ok(ContractError::TradeExpired)));
}

// ── (c) Nonce state survives a simulated contract upgrade ─────────────────────

#[test]
fn nonce_state_survives_simulated_upgrade() {
    let (env, exec_id, user, token) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 1),
        &far_future(&env),
    );

    // Simulate a contract upgrade: the ledger sequence advances substantially
    // (as it would across the deploy of a new wasm), but persistent storage —
    // and therefore the committed nonce — is untouched by an upgrade, unlike
    // instance/temporary storage which can be evicted or reset.
    env.ledger().with_mut(|l| {
        l.sequence_number += 1_000;
        l.timestamp += 5_000;
    });

    // Replaying nonce 1 post-upgrade must still be rejected.
    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 3),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::NonceAlreadyUsed)));

    // A fresh, correctly-sequenced nonce still succeeds post-upgrade.
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &2u64,
        &test_tx_hash(&env, 4),
        &far_future(&env),
    );
}

// ── purge_expired_nonces ────────────────────────────────────────────────────

#[test]
fn purge_expired_nonces_removes_only_expired_entries() {
    let (env, exec_id, user, token) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().set_timestamp(1_000);

    // Nonce 1: expires soon.
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 1),
        &2_000u64,
    );

    // Nonce 2: expires far in the future.
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &2u64,
        &test_tx_hash(&env, 2),
        &far_future(&env),
    );

    // Advance past nonce 1's expiry but not nonce 2's.
    env.ledger().set_timestamp(2_001);

    let purged = exec.purge_expired_nonces(&10u32);
    assert_eq!(purged, 1, "only the expired entry should be purged");

    // Nothing left to purge on a second pass.
    assert_eq!(exec.purge_expired_nonces(&10u32), 0);
}

#[test]
fn purge_expired_nonces_respects_max_bound() {
    let (env, exec_id, user, token) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().set_timestamp(1_000);

    for i in 1..=3u64 {
        exec.execute_copy_trade(
            &user,
            &token,
            &TRADE_AMOUNT,
            &None,
            &OrderType::Market,
            &None,
            &i,
            &test_tx_hash(&env, i as u8),
            &2_000u64,
        );
    }

    env.ledger().set_timestamp(2_001);

    // Only scan the first entry.
    let purged = exec.purge_expired_nonces(&1u32);
    assert_eq!(purged, 1);

    // The remaining two are cleaned up on a subsequent call.
    let purged_rest = exec.purge_expired_nonces(&10u32);
    assert_eq!(purged_rest, 2);
}
