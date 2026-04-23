#![cfg(test)]

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, String,
};
use stellar_swipe_common::Asset;

use crate::{
    set_pending_fees, set_treasury_balance, ContractError, FeeCollector, FeeCollectorClient,
};

#[contract]
struct MockOracleContract;

#[contractimpl]
impl MockOracleContract {
    pub fn convert_to_base(_env: Env, amount: i128, _asset: Asset) -> i128 {
        amount
    }
}

/// Helper: registers the contract, initializes it, mints tokens to it, and sets treasury balance.
fn setup(env: &Env, amount: i128) -> (Address, Address, Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let recipient = Address::generate(env);

    let token_admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(env, &token).mint(&contract_id, &amount);

    env.as_contract(&contract_id, || {
        set_treasury_balance(env, &token, amount);
    });

    (recipient, token, contract_id, client)
}

fn usd_asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "USD"),
        issuer: Some(Address::generate(env)),
    }
}

fn trade_asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "TRADE"),
        issuer: Some(Address::generate(env)),
    }
}

fn setup_oracle(env: &Env, _asset_price_in_usd: i128) -> (Address, Asset) {
    let oracle_id = env.register(MockOracleContract, ());
    let _usd = usd_asset(env);
    let asset = trade_asset(env);
    (oracle_id, asset)
}

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

#[test]
fn test_initialize_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&contract_id, &100i128);
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 100i128);
    });
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &100i128);
}

#[test]
fn test_initialize_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let result = client.try_initialize(&admin);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialized)));
}

// ---------------------------------------------------------------------------
// treasury_balance
// ---------------------------------------------------------------------------

#[test]
fn test_treasury_balance_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);

    let result = client.try_treasury_balance(&token);
    assert_eq!(result, Err(Ok(ContractError::NotInitialized)));
}

#[test]
fn test_treasury_balance_unknown_token() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    assert_eq!(client.treasury_balance(&token), 0i128);
}

// ---------------------------------------------------------------------------
// withdraw_treasury_fees
// ---------------------------------------------------------------------------

#[test]
fn test_full_balance_withdrawal() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);

    env.ledger().set_timestamp(86400);
    client.withdraw_treasury_fees(&recipient, &token, &1000i128);

    assert_eq!(client.treasury_balance(&token), 0i128);

    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&recipient), 1000i128);
}

#[test]
fn test_withdraw_insufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, contract_id, client) = setup(&env, 500i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &500i128);

    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 0i128);
    });

    env.ledger().set_timestamp(86400);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &500i128);
    assert_eq!(result, Err(Ok(ContractError::InsufficientTreasuryBalance)));
}

#[test]
fn test_withdraw_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);
    env.ledger().set_timestamp(86400);

    let non_admin = Address::generate(&env);
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;
    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "withdraw_treasury_fees",
        args: (&recipient, &token, &1000i128).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &non_admin,
        invoke: &mock_invoke,
    };
    let result = client
        .mock_auths(&[mock_auth])
        .try_withdraw_treasury_fees(&recipient, &token, &1000i128);

    assert!(result.is_err(), "non-admin call must fail");
}

#[test]
fn test_withdraw_timelock_not_elapsed() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);

    env.ledger().set_timestamp(86399);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::TimelockNotElapsed)));
}

#[test]
fn test_withdraw_not_queued() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(86400);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalNotQueued)));
}

// ---------------------------------------------------------------------------
// fee_rate / set_fee_rate
// ---------------------------------------------------------------------------

#[test]
fn test_fee_rate_default() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    assert_eq!(client.fee_rate(), 30u32);
}

#[test]
fn test_set_fee_rate_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_rate(&50u32);
    assert_eq!(client.fee_rate(), 50u32);
}

#[test]
fn test_set_fee_rate_min_boundary() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_rate(&1u32);
    assert_eq!(client.fee_rate(), 1u32);
}

#[test]
fn test_set_fee_rate_max_boundary() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_rate(&100u32);
    assert_eq!(client.fee_rate(), 100u32);
}

#[test]
fn test_set_fee_rate_too_high() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let result = client.try_set_fee_rate(&101u32);
    assert_eq!(result, Err(Ok(ContractError::FeeRateTooHigh)));
}

#[test]
fn test_set_fee_rate_too_low() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let result = client.try_set_fee_rate(&0u32);
    assert_eq!(result, Err(Ok(ContractError::FeeRateTooLow)));
}

#[test]
fn test_set_fee_rate_no_retroactive_application() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let rate_before = client.fee_rate();
    client.set_fee_rate(&75u32);

    assert_ne!(rate_before, 75u32);
    assert_eq!(client.fee_rate(), 75u32);
}

#[test]
fn test_set_fee_rate_emits_event() {
    use soroban_sdk::testutils::Events;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    env.events().all();
    client.set_fee_rate(&60u32);

    let events = env.events().all();
    assert!(!events.is_empty(), "FeeRateUpdated event must be emitted");
}

#[test]
fn test_set_fee_rate_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);

    let result = client.try_set_fee_rate(&30u32);
    assert_eq!(result, Err(Ok(ContractError::NotInitialized)));
}

#[test]
fn test_set_fee_rate_unauthorized() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "set_fee_rate",
        args: (&50u32,).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &non_admin,
        invoke: &mock_invoke,
    };
    let result = client.mock_auths(&[mock_auth]).try_set_fee_rate(&50u32);

    assert!(result.is_err(), "non-admin call to set_fee_rate must fail");
}

#[test]
fn test_collect_fee_tracks_volume_and_applies_rebate_tiers() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let (oracle_id, asset) = setup_oracle(&env, 10_000_000);
    client.set_oracle_contract(&oracle_id);
    client.set_fee_rate(&30u32);

    StellarAssetClient::new(&env, &token).mint(&trader, &(100_000 * 10_000_000));

    let fee_one = client.collect_fee(&trader, &token, &(9_000 * 10_000_000), &asset);
    assert_eq!(fee_one, 270_000_000);
    assert_eq!(client.monthly_trade_volume(&trader), 9_000 * 10_000_000);
    assert_eq!(client.fee_rate_for_user(&trader), 30u32);

    let fee_two = client.collect_fee(&trader, &token, &(2_000 * 10_000_000), &asset);
    assert_eq!(fee_two, 60_000_000);
    assert_eq!(client.monthly_trade_volume(&trader), 11_000 * 10_000_000);
    assert_eq!(client.fee_rate_for_user(&trader), 25u32);

    let fee_three = client.collect_fee(&trader, &token, &(40_000 * 10_000_000), &asset);
    assert_eq!(fee_three, 1_000_000_000);
    assert_eq!(client.monthly_trade_volume(&trader), 51_000 * 10_000_000);
    assert_eq!(client.fee_rate_for_user(&trader), 20u32);

    assert_eq!(
        client.treasury_balance(&token),
        fee_one + fee_two + fee_three
    );
}

#[test]
fn test_monthly_volume_resets_on_new_ledger_month() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let (oracle_id, asset) = setup_oracle(&env, 10_000_000);
    client.set_oracle_contract(&oracle_id);

    StellarAssetClient::new(&env, &token).mint(&trader, &(20_000 * 10_000_000));
    client.collect_fee(&trader, &token, &(12_000 * 10_000_000), &asset);
    assert_eq!(client.fee_rate_for_user(&trader), 25u32);

    env.ledger()
        .with_mut(|ledger| ledger.sequence_number += crate::storage::LEDGERS_PER_MONTH_APPROX + 1);

    assert_eq!(client.monthly_trade_volume(&trader), 0);
    assert_eq!(client.fee_rate_for_user(&trader), 30u32);
}

#[test]
fn test_collect_fee_requires_configured_oracle() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&trader, &(1_000 * 10_000_000));
    let result = client.try_collect_fee(&trader, &token, &(1_000 * 10_000_000), &trade_asset(&env));

    assert_eq!(result, Err(Ok(ContractError::OracleNotConfigured)));
}

// ---------------------------------------------------------------------------
// claim_fees
// ---------------------------------------------------------------------------

#[test]
fn test_claim_fees_normal() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let amount: i128 = 1_000_000;

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Mint pending fees to the contract and seed storage
    StellarAssetClient::new(&env, &token_id).mint(&contract_id, &amount);
    env.as_contract(&contract_id, || {
        set_pending_fees(&env, &provider, &token_id, amount);
    });

    let claimed = client.claim_fees(&provider, &token_id);
    assert_eq!(claimed, amount);

    // Pending balance must be reset to 0
    let remaining: i128 = env.as_contract(&contract_id, || {
        crate::get_pending_fees(&env, &provider, &token_id)
    });
    assert_eq!(remaining, 0);

    // Provider must have received the tokens
    assert_eq!(TokenClient::new(&env, &token_id).balance(&provider), amount);
}

#[test]
fn test_claim_fees_zero_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // No pending fees — must return 0 without error
    let claimed = client.claim_fees(&provider, &token_id);
    assert_eq!(claimed, 0);
}

#[test]
fn test_claim_fees_unauthorized() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let attacker = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Attacker tries to claim provider's fees by providing only their own auth
    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "claim_fees",
        args: (&provider, &token_id).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &attacker,
        invoke: &mock_invoke,
    };
    let result = client
        .mock_auths(&[mock_auth])
        .try_claim_fees(&provider, &token_id);

    assert!(result.is_err(), "claim with wrong auth must fail");
}

// ---------------------------------------------------------------------------
// Fee rounding tests
// ---------------------------------------------------------------------------
//
// Strategy: floor(trade_amount * fee_rate_bps / 10_000)
// - User-favorable: trader never pays more than exact pro-rata fee.
// - No dust: remainder stays with trader, not in contract.

#[test]
fn fee_floor_exact_division() {
    // 10_000 * 30 / 10_000 = 30 exactly — no rounding needed.
    assert_eq!(crate::fee_amount_floor(10_000, 30), Some(30));
}

#[test]
fn fee_floor_rounds_down_not_up() {
    // 9_999 * 30 = 299_970; 299_970 / 10_000 = 29.997 → floor = 29
    assert_eq!(crate::fee_amount_floor(9_999, 30), Some(29));
}

#[test]
fn fee_floor_one_stroop_trade() {
    // 1 * 30 / 10_000 = 0.003 → floor = 0
    assert_eq!(crate::fee_amount_floor(1, 30), Some(0));
}

#[test]
fn fee_floor_minimum_nonzero_result() {
    // Smallest amount that yields fee >= 1 at 30 bps: ceil(10_000/30) = 334
    // 334 * 30 / 10_000 = 10_020 / 10_000 = 1
    assert_eq!(crate::fee_amount_floor(334, 30), Some(1));
    // 333 * 30 / 10_000 = 9_990 / 10_000 = 0
    assert_eq!(crate::fee_amount_floor(333, 30), Some(0));
}

#[test]
fn fee_floor_max_rate() {
    // 100 bps = 1%; 10_000 * 100 / 10_000 = 100
    assert_eq!(crate::fee_amount_floor(10_000, 100), Some(100));
}

#[test]
fn fee_floor_large_amount_no_overflow() {
    // i128::MAX / 10_000 should not overflow
    let large = i128::MAX / 10_001;
    let result = crate::fee_amount_floor(large, 100);
    assert!(result.is_some());
    assert_eq!(result.unwrap(), large * 100 / 10_000);
}

#[test]
fn fee_floor_overflow_returns_none() {
    // i128::MAX * 1 overflows checked_mul
    assert_eq!(crate::fee_amount_floor(i128::MAX, 100), None);
}

/// No dust accumulates: treasury receives exactly fee_amount, nothing more.
/// After N trades the treasury balance equals the sum of all floor-rounded fees.
#[test]
fn no_dust_accumulation_over_many_trades() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let (oracle_id, asset) = setup_oracle(&env, 1);
    client.set_oracle_contract(&oracle_id);
    client.set_fee_rate(&30u32);

    // Use an amount that does NOT divide evenly: 9_999 * 30 / 10_000 = 29 (not 29.997)
    let trade_amount: i128 = 9_999;
    let expected_fee_per_trade = crate::fee_amount_floor(trade_amount, 30).unwrap(); // = 29
    let n_trades: i128 = 1_000;

    StellarAssetClient::new(&env, &token).mint(&trader, &(trade_amount * n_trades + 1_000_000));

    let mut total_fees: i128 = 0;
    for _ in 0..n_trades {
        let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
        assert_eq!(fee, expected_fee_per_trade, "each fee must be floor-rounded");
        total_fees += fee;
    }

    // Treasury balance must equal the sum of all collected fees — no extra dust.
    assert_eq!(client.treasury_balance(&token), total_fees);
    assert_eq!(total_fees, expected_fee_per_trade * n_trades);
}

/// Rebate tiers also use floor rounding — verify the discounted rate rounds down.
#[test]
fn rebate_tier_fee_also_rounds_down() {
    // Silver tier: base 30 bps - 5 bps = 25 bps
    // 9_999 * 25 / 10_000 = 249_975 / 10_000 = 24 (floor)
    assert_eq!(crate::fee_amount_floor(9_999, 25), Some(24));
    // Gold tier: base 30 bps - 10 bps = 20 bps
    // 9_999 * 20 / 10_000 = 199_980 / 10_000 = 19 (floor)
    assert_eq!(crate::fee_amount_floor(9_999, 20), Some(19));
}

/// collect_fee returns FeeRoundedToZero when the trade is too small to produce
/// a non-zero fee — the contract does not silently accept a zero-fee trade.
#[test]
fn collect_fee_rejects_zero_fee_trade() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let (oracle_id, asset) = setup_oracle(&env, 1);
    client.set_oracle_contract(&oracle_id);
    client.set_fee_rate(&30u32);

    // 333 * 30 / 10_000 = 0 → FeeRoundedToZero
    StellarAssetClient::new(&env, &token).mint(&trader, &1_000_000);
    let result = client.try_collect_fee(&trader, &token, &333, &asset);
    assert_eq!(result, Err(Ok(ContractError::FeeRoundedToZero)));
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::StellarAssetClient,
        Address, Env,
    };

    use crate::{set_treasury_balance, FeeCollector, FeeCollectorClient};

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(100))]

        #[test]
        fn prop_timelock_enforcement(
            queued_at in 0u64..=u64::MAX - 86400,
            delta in 0u64..=86399u64,
        ) {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let recipient = Address::generate(&env);
            let token_admin = Address::generate(&env);
            let token = env.register_stellar_asset_contract_v2(token_admin).address();

            let contract_id = env.register(FeeCollector, ());
            let client = FeeCollectorClient::new(&env, &contract_id);
            client.initialize(&admin);

            StellarAssetClient::new(&env, &token).mint(&contract_id, &1000i128);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, 1000i128);
            });

            env.ledger().set_timestamp(queued_at);
            client.queue_withdrawal(&recipient, &token, &1000i128);

            env.ledger().set_timestamp(queued_at + delta);
            let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);

            prop_assert_eq!(result, Err(Ok(crate::ContractError::TimelockNotElapsed)));
        }

        #[test]
        fn prop_balance_conservation_after_withdrawal(
            b in 1i128..=10_000_000i128,
            a in 1i128..=10_000_000i128,
        ) {
            let a = a.min(b);
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let recipient = Address::generate(&env);
            let token_admin = Address::generate(&env);
            let token = env.register_stellar_asset_contract_v2(token_admin).address();

            let contract_id = env.register(FeeCollector, ());
            let client = FeeCollectorClient::new(&env, &contract_id);
            client.initialize(&admin);

            StellarAssetClient::new(&env, &token).mint(&contract_id, &b);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, b);
            });

            env.ledger().set_timestamp(0);
            client.queue_withdrawal(&recipient, &token, &a);
            env.ledger().set_timestamp(86400);
            client.withdraw_treasury_fees(&recipient, &token, &a);

            prop_assert_eq!(client.treasury_balance(&token), b - a);
        }
    }
}
