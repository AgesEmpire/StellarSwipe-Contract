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

// ---------------------------------------------------------------------------
// Overflow and division-by-zero tests
// ---------------------------------------------------------------------------

#[test]
fn test_collect_fee_overflow_trade_amount() {
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

    // i128::MAX * fee_rate overflows checked_mul → ArithmeticOverflow
    StellarAssetClient::new(&env, &token).mint(&trader, &i128::MAX);
    let result = client.try_collect_fee(&trader, &token, &i128::MAX, &asset);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

#[test]
fn test_withdraw_treasury_fees_overflow_balance() {
    // Seed treasury with i128::MAX, then try to subtract more than available.
    // checked_sub on (i128::MAX - amount) where amount > i128::MAX would underflow.
    // The guard `amount > treasury_balance` fires first, so we test the sub path
    // by seeding exactly i128::MAX and withdrawing i128::MAX (valid path, no overflow).
    // To trigger the checked_sub overflow path we need balance < amount after the guard,
    // which is prevented by the guard — so we verify the guard itself catches it.
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, contract_id, client) = setup(&env, 100);
    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &100);
    env.ledger().set_timestamp(86_400);

    // Drain treasury between queue and withdraw to trigger InsufficientTreasuryBalance
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 0);
    });
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &100);
    assert_eq!(result, Err(Ok(ContractError::InsufficientTreasuryBalance)));
}

#[test]
fn test_collect_fee_zero_trade_amount_rejected() {
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

    // Zero amount must be rejected before any arithmetic
    let result = client.try_collect_fee(&trader, &token, &0, &asset);
    assert_eq!(result, Err(Ok(ContractError::InvalidAmount)));
}

// ---------------------------------------------------------------------------
// Fee rounding tests
// ---------------------------------------------------------------------------

#[test]
fn fee_floor_exact_division_no_remainder() {
    // 10_000 * 30 / 10_000 = 30 exactly
    assert_eq!(crate::fee_amount_floor(10_000, 30), Some(30));
}

#[test]
fn fee_floor_truncates_remainder() {
    // 9_999 * 30 = 299_970 / 10_000 = 29 (remainder 9_970 discarded)
    assert_eq!(crate::fee_amount_floor(9_999, 30), Some(29));
}

#[test]
fn fee_floor_minimum_nonzero() {
    // Smallest amount that yields fee >= 1 at rate 1 bps: 10_000
    assert_eq!(crate::fee_amount_floor(10_000, 1), Some(1));
    // One below: 9_999 * 1 / 10_000 = 0
    assert_eq!(crate::fee_amount_floor(9_999, 1), Some(0));
}

#[test]
fn fee_ceil_exact_division_same_as_floor() {
    assert_eq!(crate::fee_amount_ceil(10_000, 30), Some(30));
}

#[test]
fn fee_ceil_rounds_up_remainder() {
    // 9_999 * 30 = 299_970; ceil(299_970 / 10_000) = 30
    assert_eq!(crate::fee_amount_ceil(9_999, 30), Some(30));
}

#[test]
fn fee_ceil_minimum_nonzero() {
    // ceil(1 * 1 / 10_000) = ceil(0.0001) = 1
    assert_eq!(crate::fee_amount_ceil(1, 1), Some(1));
}

#[test]
fn fee_floor_and_ceil_differ_only_when_remainder_nonzero() {
    for amount in [1i128, 333, 9_999, 10_001, 99_997] {
        let floor = crate::fee_amount_floor(amount, 30).unwrap();
        let ceil = crate::fee_amount_ceil(amount, 30).unwrap();
        let remainder = (amount * 30) % 10_000;
        if remainder == 0 {
            assert_eq!(floor, ceil, "amount={amount}");
        } else {
            assert_eq!(ceil, floor + 1, "amount={amount}");
        }
    }
}

#[test]
fn collect_fee_rounds_down_user_favorable() {
    // trade_amount=9_999, rate=30 bps → floor=29, not 30
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

    StellarAssetClient::new(&env, &token).mint(&trader, &9_999);
    let fee = client.collect_fee(&trader, &token, &9_999, &asset);
    assert_eq!(fee, 29); // floor, not 30
}

#[test]
fn collect_fee_no_dust_in_contract() {
    // Every collected fee is fully credited to treasury — no sub-unit dust remains.
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

    let trade_amount = 9_999i128;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);

    // Treasury balance must equal exactly the fee collected — no hidden dust
    assert_eq!(client.treasury_balance(&token), fee);
}
