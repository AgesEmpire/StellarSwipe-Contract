#![cfg(test)]

use super::*;
use crate::admin;
use crate::oracle;
use stellar_swipe_common::oracle::OraclePrice;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Env, Symbol,
};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let contract_id = env.register(AutoTradeContract, ());
    let admin = Address::generate(&env);
    (env, contract_id, admin)
}

fn fresh_price(env: &Env, price: i128) -> OraclePrice {
    OraclePrice {
        price,
        decimals: 0,
        timestamp: env.ledger().timestamp(),
        source: Symbol::new(env, "mock"),
    }
}

// ── add_oracle ────────────────────────────────────────────────────────────────

/// Admin can add an oracle and it appears in the whitelist.
#[test]
fn test_add_oracle_appears_in_whitelist() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        let list = oracle::get_oracle_whitelist(&env, 1);
        assert_eq!(list.len(), 1);
        assert_eq!(list.get(0).unwrap(), oracle_addr);
    });
}

/// Adding the same oracle twice is idempotent — no duplicates.
#[test]
fn test_add_oracle_idempotent() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        let list = oracle::get_oracle_whitelist(&env, 1);
        assert_eq!(list.len(), 1, "duplicate add must not grow the list");
    });
}

/// Non-admin cannot add an oracle.
#[test]
fn test_non_admin_cannot_add_oracle() {
    let (env, contract_id, admin) = setup();
    let attacker = Address::generate(&env);
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        let result = oracle::add_oracle(&env, &attacker, 1, oracle_addr);
        assert_eq!(result, Err(AutoTradeError::Unauthorized));
    });
}

/// Multiple oracles can be added for the same pair.
#[test]
fn test_add_multiple_oracles_for_same_pair() {
    let (env, contract_id, admin) = setup();
    let oracle_a = Address::generate(&env);
    let oracle_b = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_a.clone()).unwrap();
        oracle::add_oracle(&env, &admin, 1, oracle_b.clone()).unwrap();

        let list = oracle::get_oracle_whitelist(&env, 1);
        assert_eq!(list.len(), 2);
    });
}

/// Whitelists are independent per asset pair.
#[test]
fn test_whitelist_is_per_asset_pair() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        // Only whitelisted for pair 1
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        // Pair 2 whitelist must be empty
        let list2 = oracle::get_oracle_whitelist(&env, 2);
        assert_eq!(list2.len(), 0);

        // Push to pair 2 must fail
        let price = fresh_price(&env, 200);
        let result = oracle::push_price_update(&env, &oracle_addr, 2, price);
        assert_eq!(result, Err(AutoTradeError::Unauthorized));
    });
}

// ── remove_oracle ─────────────────────────────────────────────────────────────

/// Admin can remove an oracle when more than one exists.
#[test]
fn test_remove_oracle_succeeds_with_multiple() {
    let (env, contract_id, admin) = setup();
    let oracle_a = Address::generate(&env);
    let oracle_b = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_a.clone()).unwrap();
        oracle::add_oracle(&env, &admin, 1, oracle_b.clone()).unwrap();

        oracle::remove_oracle(&env, &admin, 1, oracle_a.clone()).unwrap();

        let list = oracle::get_oracle_whitelist(&env, 1);
        assert_eq!(list.len(), 1);
        assert_eq!(list.get(0).unwrap(), oracle_b);
    });
}

/// Cannot remove the last oracle for a pair — returns LastOracleForPair.
#[test]
fn test_cannot_remove_last_oracle() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        let result = oracle::remove_oracle(&env, &admin, 1, oracle_addr);
        assert_eq!(result, Err(AutoTradeError::LastOracleForPair));
    });
}

/// Non-admin cannot remove an oracle.
#[test]
fn test_non_admin_cannot_remove_oracle() {
    let (env, contract_id, admin) = setup();
    let attacker = Address::generate(&env);
    let oracle_a = Address::generate(&env);
    let oracle_b = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_a.clone()).unwrap();
        oracle::add_oracle(&env, &admin, 1, oracle_b.clone()).unwrap();

        let result = oracle::remove_oracle(&env, &attacker, 1, oracle_a);
        assert_eq!(result, Err(AutoTradeError::Unauthorized));
    });
}

/// Removing an oracle that is not in the list is a no-op (list unchanged).
#[test]
fn test_remove_nonexistent_oracle_is_noop() {
    let (env, contract_id, admin) = setup();
    let oracle_a = Address::generate(&env);
    let oracle_b = Address::generate(&env);
    let stranger = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_a.clone()).unwrap();
        oracle::add_oracle(&env, &admin, 1, oracle_b.clone()).unwrap();

        // stranger is not in the list — remove should succeed silently
        oracle::remove_oracle(&env, &admin, 1, stranger).unwrap();

        let list = oracle::get_oracle_whitelist(&env, 1);
        assert_eq!(list.len(), 2, "list must be unchanged");
    });
}

// ── push_price_update ─────────────────────────────────────────────────────────

/// Whitelisted oracle can push a price update; price is stored.
#[test]
fn test_whitelisted_oracle_can_push_price() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        let price = fresh_price(&env, 500);
        oracle::push_price_update(&env, &oracle_addr, 1, price).unwrap();

        let stored = crate::risk::get_asset_price(&env, 1);
        assert_eq!(stored, Some(500));
    });
}

/// Non-whitelisted address cannot push a price update.
#[test]
fn test_non_whitelisted_oracle_cannot_push_price() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);
    let intruder = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        let price = fresh_price(&env, 500);
        let result = oracle::push_price_update(&env, &intruder, 1, price);
        assert_eq!(result, Err(AutoTradeError::Unauthorized));
    });
}

/// Whitelisted oracle pushing a stale price is rejected.
#[test]
fn test_whitelisted_oracle_stale_price_rejected() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        let stale = OraclePrice {
            price: 100,
            decimals: 0,
            timestamp: 1, // far older than MAX_PRICE_AGE_SECS from ledger ts 1_000
            source: Symbol::new(&env, "mock"),
        };
        let result = oracle::push_price_update(&env, &oracle_addr, 1, stale);
        assert_eq!(result, Err(AutoTradeError::OracleUnavailable));
    });
}

/// After removal, a previously whitelisted oracle can no longer push prices.
#[test]
fn test_removed_oracle_cannot_push_price() {
    let (env, contract_id, admin) = setup();
    let oracle_a = Address::generate(&env);
    let oracle_b = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_a.clone()).unwrap();
        oracle::add_oracle(&env, &admin, 1, oracle_b.clone()).unwrap();

        // Remove oracle_a (oracle_b remains so the pair still has one)
        oracle::remove_oracle(&env, &admin, 1, oracle_a.clone()).unwrap();

        let price = fresh_price(&env, 300);
        let result = oracle::push_price_update(&env, &oracle_a, 1, price);
        assert_eq!(result, Err(AutoTradeError::Unauthorized));
    });
}

/// oracle_price_to_i128 with decimals=2: 10000 / 100 = 100.
#[test]
fn test_push_price_decimal_scaling() {
    let (env, contract_id, admin) = setup();
    let oracle_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        admin::init_admin(&env, admin.clone());
        oracle::add_oracle(&env, &admin, 1, oracle_addr.clone()).unwrap();

        let price = OraclePrice {
            price: 10_000,
            decimals: 2,
            timestamp: env.ledger().timestamp(),
            source: Symbol::new(&env, "mock"),
        };
        oracle::push_price_update(&env, &oracle_addr, 1, price).unwrap();

        // 10_000 / 10^2 = 100
        let stored = crate::risk::get_asset_price(&env, 1);
        assert_eq!(stored, Some(100));
    });
}
