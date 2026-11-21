#![cfg(test)]
//! Issue #1416: liquidity reserved for a message that can no longer be
//! delivered is released exactly once, and unrelated reservations are left
//! untouched.

use crate::reservations::{AssetLiquidity, ReservationStatus};
use crate::{BridgeContract, BridgeContractClient, BridgeError, ChainId, TransferStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};

const START: u64 = 1_000;
const SHORT_EXPIRY: u64 = START + 100;
const LONG_EXPIRY: u64 = START + 10_000;
const TOTAL_LIQUIDITY: i128 = 1_000;

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
    client.initialize(&admin, &validators, &2, &1_000, &10_000, &600);
    for asset in ["wETH", "wBTC"] {
        client.register_wrapped_asset(
            &admin,
            &ChainId::Ethereum,
            &String::from_str(&env, "SRC"),
            &String::from_str(&env, asset),
            &18,
        );
        client.set_bridge_asset_liquidity(
            &admin,
            &String::from_str(&env, asset),
            &TOTAL_LIQUIDITY,
        );
    }
    let user = Address::generate(&env);
    Setup {
        env,
        client,
        admin,
        validators,
        user,
    }
}

fn asset(s: &Setup, name: &str) -> String {
    String::from_str(&s.env, name)
}

fn initiate(s: &Setup, asset_name: &str, nonce: u64, amount: i128, expires_at: u64) -> u64 {
    s.client.initiate_lock_mint_with_expiry(
        &s.user,
        &ChainId::Ethereum,
        &ChainId::Polygon,
        &String::from_str(&s.env, "SRC"),
        &asset(s, asset_name),
        &amount,
        &String::from_str(&s.env, "0xreserve"),
        &nonce,
        &String::from_str(&s.env, "stellar:user"),
        &expires_at,
    )
}

fn reserved_transfer(s: &Setup, asset_name: &str, nonce: u64, amount: i128, expires_at: u64) -> u64 {
    let id = initiate(s, asset_name, nonce, amount, expires_at);
    s.client.reserve_transfer_liquidity(&s.admin, &id);
    id
}

fn liquidity(s: &Setup, name: &str) -> AssetLiquidity {
    s.client.get_bridge_asset_liquidity(&asset(s, name))
}

#[test]
fn reservation_reduces_available_liquidity() {
    let s = setup();
    let id = reserved_transfer(&s, "wETH", 1, 400, SHORT_EXPIRY);

    assert_eq!(s.client.get_available_bridge_liquidity(&asset(&s, "wETH")), 600);
    let r = s.client.get_liquidity_reservation(&id).unwrap();
    assert_eq!(r.status, ReservationStatus::Active);
    assert_eq!(r.amount, 400);
    assert_eq!(r.expires_at, SHORT_EXPIRY);
}

#[test]
fn reservation_cannot_exceed_available_liquidity() {
    let s = setup();
    reserved_transfer(&s, "wETH", 1, 700, SHORT_EXPIRY);
    let id = initiate(&s, "wETH", 2, 400, SHORT_EXPIRY);
    assert_eq!(
        s.client.try_reserve_transfer_liquidity(&s.admin, &id),
        Err(Ok(BridgeError::InsufficientBridgeLiquidity))
    );
    assert!(s.client.get_liquidity_reservation(&id).is_none());
    assert_eq!(liquidity(&s, "wETH").reserved, 700);
}

#[test]
fn not_yet_expired_reservation_is_not_released() {
    let s = setup();
    let id = reserved_transfer(&s, "wETH", 1, 400, SHORT_EXPIRY);

    s.env.ledger().set_timestamp(SHORT_EXPIRY - 1);
    assert_eq!(s.client.release_expired_reservation(&id), false);
    assert_eq!(s.client.release_expired_reservations(&10), 0);
    assert_eq!(liquidity(&s, "wETH").reserved, 400);
    assert_eq!(
        s.client.get_liquidity_reservation(&id).unwrap().status,
        ReservationStatus::Active
    );
}

#[test]
fn expired_reservation_is_released_and_liquidity_becomes_available() {
    let s = setup();
    let id = reserved_transfer(&s, "wETH", 1, 400, SHORT_EXPIRY);

    s.env.ledger().set_timestamp(SHORT_EXPIRY);
    assert_eq!(s.client.release_expired_reservations(&10), 1);

    assert_eq!(
        liquidity(&s, "wETH"),
        AssetLiquidity {
            total: TOTAL_LIQUIDITY,
            reserved: 0
        }
    );
    assert_eq!(
        s.client.get_available_bridge_liquidity(&asset(&s, "wETH")),
        TOTAL_LIQUIDITY
    );
    let r = s.client.get_liquidity_reservation(&id).unwrap();
    assert_eq!(r.status, ReservationStatus::Released);
    assert_eq!(r.settled_at, Some(SHORT_EXPIRY));

    // The released liquidity can back a new reservation in full.
    let next = reserved_transfer(&s, "wETH", 2, TOTAL_LIQUIDITY, SHORT_EXPIRY + 500);
    assert_eq!(
        s.client.get_liquidity_reservation(&next).unwrap().status,
        ReservationStatus::Active
    );
    assert_eq!(s.client.get_available_bridge_liquidity(&asset(&s, "wETH")), 0);
}

#[test]
fn repeated_expiry_processing_is_idempotent() {
    let s = setup();
    let id = reserved_transfer(&s, "wETH", 1, 400, SHORT_EXPIRY);

    s.env.ledger().set_timestamp(SHORT_EXPIRY + 1);
    assert_eq!(s.client.release_expired_reservation(&id), true);
    let after_first = liquidity(&s, "wETH");
    let reservation_after_first = s.client.get_liquidity_reservation(&id).unwrap();

    // Single-target and batch processing both become no-ops.
    assert_eq!(s.client.release_expired_reservation(&id), false);
    assert_eq!(s.client.release_expired_reservations(&10), 0);
    s.env.ledger().set_timestamp(SHORT_EXPIRY + 5_000);
    assert_eq!(s.client.release_expired_reservations(&10), 0);
    assert_eq!(s.client.release_expired_reservation(&id), false);

    assert_eq!(liquidity(&s, "wETH"), after_first);
    assert_eq!(
        s.client.get_liquidity_reservation(&id).unwrap(),
        reservation_after_first
    );
}

#[test]
fn unrelated_reservations_remain_unchanged() {
    let s = setup();
    let expiring = reserved_transfer(&s, "wETH", 1, 300, SHORT_EXPIRY);
    let same_asset_live = reserved_transfer(&s, "wETH", 2, 200, LONG_EXPIRY);
    let other_asset_live = reserved_transfer(&s, "wBTC", 3, 500, LONG_EXPIRY);

    let live_before = s.client.get_liquidity_reservation(&same_asset_live).unwrap();
    let other_before = s.client.get_liquidity_reservation(&other_asset_live).unwrap();
    let wbtc_before = liquidity(&s, "wBTC");

    s.env.ledger().set_timestamp(SHORT_EXPIRY);
    assert_eq!(s.client.release_expired_reservations(&10), 1);

    assert_eq!(
        s.client.get_liquidity_reservation(&expiring).unwrap().status,
        ReservationStatus::Released
    );
    assert_eq!(
        s.client.get_liquidity_reservation(&same_asset_live).unwrap(),
        live_before
    );
    assert_eq!(
        s.client.get_liquidity_reservation(&other_asset_live).unwrap(),
        other_before
    );
    // Only the expired reservation's amount left the wETH reserved total.
    assert_eq!(liquidity(&s, "wETH").reserved, 200);
    assert_eq!(liquidity(&s, "wBTC"), wbtc_before);
}

#[test]
fn executed_transfer_consumes_reservation_and_is_never_released() {
    let s = setup();
    let id = reserved_transfer(&s, "wETH", 1, 400, SHORT_EXPIRY);
    s.client
        .approve_lock_mint(&s.validators.get(0).unwrap(), &id, &String::from_str(&s.env, "sig-a"));
    s.client
        .approve_lock_mint(&s.validators.get(1).unwrap(), &id, &String::from_str(&s.env, "sig-b"));
    s.client.execute_lock_mint(&s.admin, &id);
    assert_eq!(s.client.get_transfer(&id).status, TransferStatus::Completed);

    let r = s.client.get_liquidity_reservation(&id).unwrap();
    assert_eq!(r.status, ReservationStatus::Consumed);
    assert_eq!(
        liquidity(&s, "wETH"),
        AssetLiquidity {
            total: TOTAL_LIQUIDITY - 400,
            reserved: 0
        }
    );

    // After the original expiry, processing does not hand the liquidity back.
    s.env.ledger().set_timestamp(SHORT_EXPIRY + 1);
    assert_eq!(s.client.release_expired_reservations(&10), 0);
    assert_eq!(s.client.release_expired_reservation(&id), false);
    assert_eq!(liquidity(&s, "wETH").total, TOTAL_LIQUIDITY - 400);
}

#[test]
fn permanently_failed_transfer_releases_before_expiry() {
    let s = setup();
    let id = reserved_transfer(&s, "wETH", 1, 400, LONG_EXPIRY);
    s.client
        .mark_transfer_failed(&s.admin, &id, &String::from_str(&s.env, "bad route"), &false);

    assert_eq!(s.client.release_expired_reservation(&id), true);
    assert_eq!(liquidity(&s, "wETH").reserved, 0);
    assert_eq!(s.client.release_expired_reservation(&id), false);
}

#[test]
fn batch_size_is_bounded() {
    let s = setup();
    assert_eq!(
        s.client.try_release_expired_reservations(&0),
        Err(Ok(BridgeError::InvalidAmount))
    );
    assert_eq!(
        s.client
            .try_release_expired_reservations(&(crate::reservations::MAX_EXPIRY_BATCH + 1)),
        Err(Ok(BridgeError::InvalidAmount))
    );

    for nonce in 1..=3u64 {
        reserved_transfer(&s, "wETH", nonce, 100, SHORT_EXPIRY);
    }
    s.env.ledger().set_timestamp(SHORT_EXPIRY);
    assert_eq!(s.client.release_expired_reservations(&2), 2);
    assert_eq!(liquidity(&s, "wETH").reserved, 100);
    assert_eq!(s.client.release_expired_reservations(&2), 1);
    assert_eq!(liquidity(&s, "wETH").reserved, 0);
    assert_eq!(s.client.release_expired_reservations(&2), 0);
}
