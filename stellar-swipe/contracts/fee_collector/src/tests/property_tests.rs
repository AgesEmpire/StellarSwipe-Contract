#![cfg(test)]

use crate::{fee_amount_floor, FeeCollector, FeeCollectorClient};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};
use stellar_swipe_common::Asset;

fn setup_contract(env: &Env) -> FeeCollectorClient<'_> {
    let admin = Address::generate(env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);
    client
}

fn trade_asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "TRADE"),
        issuer: Some(Address::generate(env)),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

    #[test]
    fn fee_plus_net_equals_gross(trade_amount in 1_i128..=1_000_000_000_000_i128, fee_rate_bps in crate::storage::MIN_FEE_RATE_BPS..=crate::storage::MAX_FEE_RATE_BPS) {
        let fee = fee_amount_floor(trade_amount, fee_rate_bps).expect("fee calculation should not overflow");
        let net_amount = trade_amount - fee;

        prop_assert_eq!(fee + net_amount, trade_amount);
    }

    #[test]
    fn fee_is_at_least_minimum_bound(trade_amount in 1_i128..=1_000_000_000_000_i128, fee_rate_bps in crate::storage::MIN_FEE_RATE_BPS..=crate::storage::MAX_FEE_RATE_BPS) {
        let min_fee = fee_amount_floor(trade_amount, crate::storage::MIN_FEE_RATE_BPS).unwrap_or(0);
        let fee = fee_amount_floor(trade_amount, fee_rate_bps).expect("fee calculation should not overflow");

        prop_assert!(fee >= min_fee);
    }

    #[test]
    fn fee_is_at_most_maximum_bound(trade_amount in 1_i128..=1_000_000_000_000_i128, fee_rate_bps in crate::storage::MIN_FEE_RATE_BPS..=crate::storage::MAX_FEE_RATE_BPS) {
        let fee = fee_amount_floor(trade_amount, fee_rate_bps).expect("fee calculation should not overflow");
        let max_fee = fee_amount_floor(trade_amount, crate::storage::MAX_FEE_RATE_BPS).unwrap_or(0);

        prop_assert!(fee <= max_fee);
    }

    #[test]
    fn fee_exempt_user_pays_zero(trade_amount in 1_i128..=1_000_000_000_000_i128) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|ledger| ledger.timestamp = 0);

        let trader = Address::generate(&env);
        let token = Address::generate(&env);
        let asset = trade_asset(&env);
        let client = setup_contract(&env);

        let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);

        prop_assert_eq!(fee, 0);
    }
}
