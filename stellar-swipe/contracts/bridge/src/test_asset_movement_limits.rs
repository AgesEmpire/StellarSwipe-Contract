#![cfg(test)]
//! Issue #1417: configurable per-asset aggregate bridge movement limits over
//! a ledger-time window.

use crate::movement_limits::{AssetMovementLimit, AssetMovementWindow};
use crate::{BridgeContract, BridgeContractClient, BridgeError, ChainId};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};

/// Aligned to a 3_600s window so `START` is the first second of a bucket.
const START: u64 = 36_000;
const WINDOW: u64 = 3_600;
const LIMIT: i128 = 500;

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
    client.initialize(&admin, &validators, &2, &10_000, &100_000, &0);
    for asset in ["wETH", "wBTC"] {
        client.register_wrapped_asset(
            &admin,
            &ChainId::Ethereum,
            &String::from_str(&env, "SRC"),
            &String::from_str(&env, asset),
            &18,
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

fn lock_mint(s: &Setup, asset_name: &str, nonce: u64, amount: i128) -> Result<u64, BridgeError> {
    match s.client.try_initiate_lock_mint(
        &s.user,
        &ChainId::Ethereum,
        &ChainId::Polygon,
        &String::from_str(&s.env, "SRC"),
        &asset(s, asset_name),
        &amount,
        &String::from_str(&s.env, "0xmove"),
        &nonce,
        &String::from_str(&s.env, "stellar:user"),
    ) {
        Ok(Ok(id)) => Ok(id),
        Err(Ok(e)) => Err(e),
        other => panic!("unexpected result: {:?}", other),
    }
}

fn burn_unlock(s: &Setup, asset_name: &str, amount: i128) -> Result<u64, BridgeError> {
    match s.client.try_initiate_burn_unlock(
        &s.user,
        &ChainId::Polygon,
        &ChainId::Ethereum,
        &String::from_str(&s.env, "SRC"),
        &asset(s, asset_name),
        &amount,
        &String::from_str(&s.env, "0xrecipient"),
    ) {
        Ok(Ok(id)) => Ok(id),
        Err(Ok(e)) => Err(e),
        other => panic!("unexpected result: {:?}", other),
    }
}

/// Mint `amount` of `asset_name` to the user through the full lock/mint flow.
fn mint(s: &Setup, asset_name: &str, nonce: u64, amount: i128) {
    let id = lock_mint(s, asset_name, nonce, amount).unwrap();
    s.client
        .approve_lock_mint(&s.validators.get(0).unwrap(), &id, &String::from_str(&s.env, "sig-a"));
    s.client
        .approve_lock_mint(&s.validators.get(1).unwrap(), &id, &String::from_str(&s.env, "sig-b"));
    s.client.execute_lock_mint(&s.admin, &id);
}

fn used(s: &Setup, name: &str) -> i128 {
    s.client.get_asset_movement_usage(&asset(s, name)).total_moved
}

#[test]
fn movement_up_to_limit_is_allowed_and_crossing_fails() {
    let s = setup();
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &LIMIT, &WINDOW);

    assert!(lock_mint(&s, "wETH", 1, 300).is_ok());
    assert!(lock_mint(&s, "wETH", 2, 200).is_ok());
    assert_eq!(used(&s, "wETH"), LIMIT);

    let next_id = s.client.get_bridge_config().next_transfer_id;
    assert_eq!(
        lock_mint(&s, "wETH", 3, 1),
        Err(BridgeError::AssetMovementLimitExceeded)
    );
    // Rejected before any bridge state was written.
    assert_eq!(s.client.get_bridge_config().next_transfer_id, next_id);
    assert_eq!(used(&s, "wETH"), LIMIT);
    assert_eq!(
        s.client.try_get_transfer(&next_id),
        Err(Ok(BridgeError::TransferNotFound))
    );
    // The replay lock for the rejected message was not written either.
    s.client.remove_asset_movement_limit(&s.admin, &asset(&s, "wETH"));
    assert!(lock_mint(&s, "wETH", 3, 1).is_ok());
}

#[test]
fn limits_are_asset_scoped() {
    let s = setup();
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &LIMIT, &WINDOW);

    assert!(lock_mint(&s, "wETH", 1, LIMIT).is_ok());
    assert_eq!(
        lock_mint(&s, "wETH", 2, 1),
        Err(BridgeError::AssetMovementLimitExceeded)
    );
    // An unlimited asset is unaffected by wETH's exhausted window.
    assert!(lock_mint(&s, "wBTC", 3, 2_000).is_ok());
    assert_eq!(used(&s, "wBTC"), 0);

    // A separate limit on wBTC accumulates independently.
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wBTC"), &100, &WINDOW);
    assert!(lock_mint(&s, "wBTC", 4, 100).is_ok());
    assert_eq!(used(&s, "wBTC"), 100);
    assert_eq!(used(&s, "wETH"), LIMIT);
}

#[test]
fn burn_unlock_over_limit_fails_before_balance_changes() {
    let s = setup();
    mint(&s, "wETH", 1, 1_000);
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &LIMIT, &WINDOW);

    let balance_before = s.client.get_wrapped_balance(&s.user, &asset(&s, "wETH"));
    let minted_before = s.client.get_total_minted();
    let next_id = s.client.get_bridge_config().next_transfer_id;

    assert_eq!(
        burn_unlock(&s, "wETH", LIMIT + 1),
        Err(BridgeError::AssetMovementLimitExceeded)
    );
    assert_eq!(
        s.client.get_wrapped_balance(&s.user, &asset(&s, "wETH")),
        balance_before
    );
    assert_eq!(s.client.get_total_minted(), minted_before);
    assert_eq!(s.client.get_bridge_config().next_transfer_id, next_id);
    assert_eq!(used(&s, "wETH"), 0);

    // Within the limit the burn proceeds and is accounted.
    assert!(burn_unlock(&s, "wETH", LIMIT).is_ok());
    assert_eq!(
        s.client.get_wrapped_balance(&s.user, &asset(&s, "wETH")),
        balance_before - LIMIT
    );
    assert_eq!(used(&s, "wETH"), LIMIT);
}

#[test]
fn window_rollover_resets_usage() {
    let s = setup();
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &LIMIT, &WINDOW);
    assert!(lock_mint(&s, "wETH", 1, LIMIT).is_ok());

    // Last second of the window: still exhausted.
    s.env.ledger().set_timestamp(START + WINDOW - 1);
    assert_eq!(
        lock_mint(&s, "wETH", 2, 1),
        Err(BridgeError::AssetMovementLimitExceeded)
    );

    // First second of the next aligned window: fresh budget.
    s.env.ledger().set_timestamp(START + WINDOW);
    assert_eq!(
        s.client.get_asset_movement_usage(&asset(&s, "wETH")),
        AssetMovementWindow {
            window_start: START + WINDOW,
            total_moved: 0
        }
    );
    assert!(lock_mint(&s, "wETH", 2, LIMIT).is_ok());
    assert_eq!(used(&s, "wETH"), LIMIT);
    assert_eq!(
        lock_mint(&s, "wETH", 3, 1),
        Err(BridgeError::AssetMovementLimitExceeded)
    );
}

#[test]
fn admin_limit_update_keeps_current_window_volume() {
    let s = setup();
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &LIMIT, &WINDOW);
    assert!(lock_mint(&s, "wETH", 1, 300).is_ok());

    // Lowering the limit applies immediately against volume already moved.
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &350, &WINDOW);
    assert_eq!(used(&s, "wETH"), 300);
    assert_eq!(
        lock_mint(&s, "wETH", 2, 51),
        Err(BridgeError::AssetMovementLimitExceeded)
    );
    assert!(lock_mint(&s, "wETH", 2, 50).is_ok());

    // Raising it lets more through in the same window.
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &1_000, &WINDOW);
    assert!(lock_mint(&s, "wETH", 3, 650).is_ok());
    assert_eq!(used(&s, "wETH"), 1_000);
    assert_eq!(
        s.client.get_asset_movement_limit(&asset(&s, "wETH")),
        Some(AssetMovementLimit {
            limit: 1_000,
            window_seconds: WINDOW
        })
    );
}

#[test]
fn admin_window_change_starts_fresh_window() {
    let s = setup();
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &LIMIT, &WINDOW);
    assert!(lock_mint(&s, "wETH", 1, LIMIT).is_ok());

    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &LIMIT, &(2 * WINDOW));
    assert_eq!(used(&s, "wETH"), 0);
    assert!(lock_mint(&s, "wETH", 2, LIMIT).is_ok());
    assert_eq!(
        lock_mint(&s, "wETH", 3, 1),
        Err(BridgeError::AssetMovementLimitExceeded)
    );
}

#[test]
fn invalid_or_unauthorized_updates_are_rejected() {
    let s = setup();
    let weth = asset(&s, "wETH");
    assert_eq!(
        s.client.try_set_asset_movement_limit(&s.admin, &weth, &0, &WINDOW),
        Err(Ok(BridgeError::InvalidAmount))
    );
    assert_eq!(
        s.client.try_set_asset_movement_limit(&s.admin, &weth, &LIMIT, &0),
        Err(Ok(BridgeError::InvalidAmount))
    );
    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.client.try_set_asset_movement_limit(&stranger, &weth, &LIMIT, &WINDOW),
        Err(Ok(BridgeError::UnauthorizedValidator))
    );
    assert_eq!(s.client.get_asset_movement_limit(&weth), None);
}

#[test]
fn accumulation_overflow_is_rejected_as_limit_breach() {
    let s = setup();
    s.client
        .set_asset_movement_limit(&s.admin, &asset(&s, "wETH"), &i128::MAX, &WINDOW);
    let contract_id = s.client.address.clone();
    s.env.as_contract(&contract_id, || {
        let wnd = AssetMovementWindow {
            window_start: START,
            total_moved: i128::MAX,
        };
        crate::movement_limits::commit_movement(&s.env, &asset(&s, "wETH"), &wnd);
        assert_eq!(
            crate::movement_limits::check_movement(&s.env, &asset(&s, "wETH"), 1),
            Err(BridgeError::AssetMovementLimitExceeded)
        );
    });
}
