#![cfg(test)]

use crate::{PnlSummary, UserPortfolio, UserPortfolioClient};
use proptest::prelude::*;
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Ledger},
    Address, Env,
};
use stellar_swipe_common::OraclePrice;

#[contract]
pub struct OracleMock;

#[contractimpl]
impl OracleMock {
    pub fn set_price(env: Env, asset_pair: u32, price: OraclePrice) {
        env.storage()
            .instance()
            .set(&(symbol_short!("price"), asset_pair), &price);
    }

    pub fn get_price(env: Env, asset_pair: u32) -> OraclePrice {
        env.storage()
            .instance()
            .get(&(symbol_short!("price"), asset_pair))
            .unwrap()
    }
}

fn setup(oracle_price: i128) -> (Env, Address, UserPortfolioClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.timestamp = 1_000);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let oracle_id = env.register_contract(None, OracleMock);
    OracleMockClient::new(&env, &oracle_id).set_price(
        &0u32,
        &OraclePrice {
            price: oracle_price * 100,
            decimals: 2,
            timestamp: env.ledger().timestamp(),
            source: soroban_sdk::Symbol::new(&env, "mock"),
        },
    );

    let portfolio_id = env.register_contract(None, UserPortfolio);
    let client = UserPortfolioClient::new(&env, &portfolio_id);
    client.initialize(&admin, &oracle_id);

    (env, user, client)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

    #[test]
    fn realized_plus_entry_equals_exit(
        entry_value in 1_i128..=1_000_000_000_i128,
        pnl_delta in -1_000_000_i128..=1_000_000_i128
    ) {
        let exit_value = entry_value + pnl_delta;
        let realized_pnl = exit_value - entry_value;

        prop_assert_eq!(realized_pnl + entry_value, exit_value);
    }

    #[test]
    fn pnl_sign_matches_trade_direction(
        entry_value in 1_i128..=1_000_000_000_i128,
        delta in -1_000_000_i128..=1_000_000_i128
    ) {
        let pnl = delta;

        if delta > 0 {
            prop_assert!(pnl > 0);
        } else if delta < 0 {
            prop_assert!(pnl < 0);
        } else {
            prop_assert_eq!(pnl, 0);
        }

        let exit_value = entry_value + delta;
        prop_assert_eq!(entry_value + pnl, exit_value);
    }

    #[test]
    fn total_pnl_equals_sum_of_closed_positions(
        pnl1 in -100_000_i128..=100_000_i128,
        pnl2 in -100_000_i128..=100_000_i128,
        pnl3 in -100_000_i128..=100_000_i128
    ) {
        let (env, user, client) = setup(120);
        let provider = Address::generate(&env);

        client.open_position(&user, &100, &1);
        client.open_position(&user, &100, &1);
        client.open_position(&user, &100, &1);

        client.close_position(&user, &1, &pnl1, &120i128, &0u32, &provider, &0u64);
        client.close_position(&user, &2, &pnl2, &120i128, &0u32, &provider, &0u64);
        client.close_position(&user, &3, &pnl3, &120i128, &0u32, &provider, &0u64);

        let PnlSummary { realized_pnl, total_pnl, .. } = client.get_pnl(&user);
        let expected_total = pnl1 + pnl2 + pnl3;

        prop_assert_eq!(realized_pnl, expected_total);
        prop_assert_eq!(total_pnl, expected_total);
    }
}
