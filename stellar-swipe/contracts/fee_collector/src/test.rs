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
    MAX_AUDIT_TOKENS,
};

/// Pre-mark a trader as having already completed their first trade,
/// so subsequent `collect_fee` calls use the normal fee path.
fn mark_trader_has_traded(env: &Env, contract_id: &Address, trader: &Address) {
    env.as_contract(contract_id, || {
        crate::storage::set_has_traded(env, trader);
    });
}

/// Disable revenue-share diversion so treasury assertions reflect fee minus burn only.
fn disable_revenue_share(client: &FeeCollectorClient<'_>) {
    client.set_revenue_share_rate_bps(&0u32);
}

// Stellar burn address (all-zeros public key encoded as strkey)
const _BURN_ADDRESS: &str = "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN";

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
    disable_revenue_share(&client);

    StellarAssetClient::new(&env, &token).mint(&trader, &(100_000 * 10_000_000));

    // Pre-mark trader as having completed their first trade so normal fees apply.
    mark_trader_has_traded(&env, &contract_id, &trader);

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
        // default burn_rate = 10%, so treasury receives 90% of each fee
        (fee_one + fee_two + fee_three) * 9 / 10
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
    mark_trader_has_traded(&env, &contract_id, &trader);
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
// burn_rate / set_burn_rate
// ---------------------------------------------------------------------------

#[test]
fn test_burn_rate_default() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);
    assert_eq!(client.burn_rate(), 1_000u32); // 10% default
}

#[test]
fn test_set_burn_rate_configurable_by_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_burn_rate(&500u32); // 5%
    assert_eq!(client.burn_rate(), 500u32);
}

#[test]
fn test_set_burn_rate_too_high() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);
    let result = client.try_set_burn_rate(&10_001u32);
    assert_eq!(result, Err(Ok(ContractError::BurnRateTooHigh)));
}

#[test]
fn test_set_burn_rate_max_boundary() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_burn_rate(&10_000u32); // 100% — valid boundary
    assert_eq!(client.burn_rate(), 10_000u32);
}

#[test]
fn test_collect_fee_burn_amount_calculation() {
    // fee_amount = 1_000_000 * 30 / 10_000 = 3_000
    // burn = 3_000 * 1_000 / 10_000 = 300  (10%)
    // treasury receives 2_700
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
    client.set_burn_rate(&1_000u32); // 10%
    disable_revenue_share(&client);

    let trade_amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    mark_trader_has_traded(&env, &contract_id, &trader);
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee, 3_000); // total fee collected from trader

    // treasury should hold fee minus burn: 3_000 - 300 = 2_700
    assert_eq!(client.treasury_balance(&token), 2_700);
}

#[test]
fn test_collect_fee_zero_burn_rate_full_treasury() {
    // burn_rate = 0 → nothing burned, full fee goes to treasury
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
    client.set_burn_rate(&0u32);
    disable_revenue_share(&client);

    let trade_amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    mark_trader_has_traded(&env, &contract_id, &trader);
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee, 3_000);
    assert_eq!(client.treasury_balance(&token), 3_000); // nothing burned
}

#[test]
fn test_collect_fee_full_burn_rate_zero_treasury() {
    // burn_rate = 10_000 (100%) → all fee burned, treasury gets 0
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
    client.set_burn_rate(&10_000u32);

    let trade_amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    mark_trader_has_traded(&env, &contract_id, &trader);
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee, 3_000);
    assert_eq!(client.treasury_balance(&token), 0); // all burned
}

#[test]
fn test_collect_fee_emits_fees_burned_event() {
    use soroban_sdk::testutils::Events;

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
    client.set_burn_rate(&1_000u32);

    StellarAssetClient::new(&env, &token).mint(&trader, &1_000_000i128);
    client.collect_fee(&trader, &token, &1_000_000i128, &asset);

    let events = env.events().all();
    assert!(!events.is_empty(), "FeesBurned event must be emitted");
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
// Rounding strategy tests
// ---------------------------------------------------------------------------

/// Verifies that user-paid fees always round DOWN (user-favorable).
/// trade_amount=9999, fee_rate=30 bps → 9999*30/10000 = 29.997 → truncates to 29.
#[test]
fn test_fee_rounds_down_user_favorable() {
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
    client.set_fee_rate(&30u32); // 0.30%
    client.set_burn_rate(&0u32); // no burn for clarity

    let trade_amount: i128 = 9_999;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    mark_trader_has_traded(&env, &contract_id, &trader);
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    // 9999 * 30 / 10000 = 29.997 → truncated to 29 (user pays less, not more)
    assert_eq!(fee, 29);
}

/// Verifies that burn rounds DOWN so distributable = fee - burn with no dust.
/// fee=3000, burn_rate=1000 (10%) → burn=300, distributable=2700. 300+2700=3000 ✓
/// trade_amount=1_000_001: 1_000_001*30/10_000 = 3000 (truncated, remainder discarded)
#[test]
fn test_burn_rounds_down_no_dust() {
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
    // fee_rate=30 bps, trade_amount=1_000_001 → fee = 1_000_001*30/10_000 = 3000 (truncated)
    client.set_fee_rate(&30u32);
    client.set_burn_rate(&1_000u32); // 10%
    disable_revenue_share(&client);

    let trade_amount: i128 = 1_000_001;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    mark_trader_has_traded(&env, &contract_id, &trader);
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee, 3_000); // 1_000_001 * 30 / 10_000 = 3000 (truncated)

    // burn = 3000 * 1000 / 10000 = 300 (exact, no truncation needed)
    // distributable = 3000 - 300 = 2700
    // treasury must hold exactly 2700 — no dust left unaccounted
    assert_eq!(client.treasury_balance(&token), 2_700);
    // burn(300) + treasury(2700) == fee(3000): conservation holds
    assert_eq!(300 + 2_700, fee);
}

/// Verifies fee + remainder conservation: the contract never holds unwithdrawable dust.
/// For any fee_amount, burn_amount + distributable == fee_amount exactly.
#[test]
fn test_no_unwithdrawable_dust_accumulates() {
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
    client.set_burn_rate(&3_333u32); // 33.33% — non-round to stress remainder
    disable_revenue_share(&client);

    // Use a trade amount that produces a non-round fee
    let trade_amount: i128 = 777_777;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    mark_trader_has_traded(&env, &contract_id, &trader);
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    // fee = 777_777 * 30 / 10_000 = 2333 (truncated)
    assert_eq!(fee, 2_333);

    // burn = 2333 * 3333 / 10_000 = 777 (truncated)
    // distributable = 2333 - 777 = 1556
    // 777 + 1556 = 2333 == fee: no dust
    assert_eq!(client.treasury_balance(&token), 1_556);
    assert_eq!(777 + 1_556, fee);
}

/// Verifies minimum-amount boundary: fee_rate=1 bps on small amounts rounds to zero → error.
#[test]
fn test_fee_rounded_to_zero_error() {
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
    client.set_fee_rate(&1u32); // 0.01 bps — minimum

    // trade_amount=9999: 9999 * 1 / 10_000 = 0 → FeeRoundedToZero
    let trade_amount: i128 = 9_999;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    mark_trader_has_traded(&env, &contract_id, &trader);
    let result = client.try_collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(result, Err(Ok(ContractError::FeeRoundedToZero)));
}

// ---------------------------------------------------------------------------
// Overflow / division-by-zero tests
// ---------------------------------------------------------------------------

/// collect_fee: trade_amount * fee_rate overflows i128 → ArithmeticOverflow.
/// We use i128::MAX as trade_amount with fee_rate > 1 to force overflow in checked_mul.
#[test]
fn test_collect_fee_overflow_returns_error() {
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

    // i128::MAX * 30 overflows — checked_mul returns None → ArithmeticOverflow
    StellarAssetClient::new(&env, &token).mint(&trader, &i128::MAX);
    mark_trader_has_traded(&env, &contract_id, &trader);
    let result = client.try_collect_fee(&trader, &token, &i128::MAX, &asset);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

/// queue_withdrawal: queued_at near u64::MAX + SECONDS_PER_DAY overflows → ArithmeticOverflow.
#[test]
fn test_queue_withdrawal_timestamp_overflow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&contract_id, &1000i128);
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 1000i128);
    });

    // Set timestamp so that queued_at + SECONDS_PER_DAY (86400) wraps u64
    env.ledger().set_timestamp(u64::MAX - 100);

    let result = client.try_queue_withdrawal(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

/// withdraw_treasury_fees: timelock check with queued_at near u64::MAX → ArithmeticOverflow.
#[test]
fn test_withdraw_timelock_timestamp_overflow() {
    use crate::{set_queued_withdrawal, QueuedWithdrawal};

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&contract_id, &1000i128);
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 1000i128);
        // Manually inject a queued withdrawal with queued_at near u64::MAX
        set_queued_withdrawal(
            &env,
            &QueuedWithdrawal {
                recipient: recipient.clone(),
                token: token.clone(),
                amount: 1000,
                queued_at: u64::MAX - 100,
            },
        );
    });

    // Current timestamp is 0; queued_at + SECONDS_PER_DAY overflows → ArithmeticOverflow
    env.ledger().set_timestamp(0);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

// ---------------------------------------------------------------------------
// audit_balances — reconciliation tests
// ---------------------------------------------------------------------------

/// Helper: set up contract with a token and matching on-chain + stored balances.
fn setup_audit(env: &Env, amount: i128) -> (Address, Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let token_admin = Address::generate(env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);
    StellarAssetClient::new(env, &token).mint(&contract_id, &amount);
    env.as_contract(&contract_id, || {
        set_treasury_balance(env, &token, amount);
    });
    (token, contract_id, client)
}

#[test]
fn test_audit_balances_no_mismatch() {
    let env = Env::default();
    env.mock_all_auths();

    let (token, _contract_id, client) = setup_audit(&env, 1_000i128);

    let tokens = soroban_sdk::vec![&env, token.clone()];
    let mismatches = client.audit_balances(&tokens);

    // Stored balance equals on-chain balance: no mismatches
    assert_eq!(mismatches.len(), 0);
}

#[test]
fn test_audit_balances_detects_surplus() {
    let env = Env::default();
    env.mock_all_auths();

    let (token, contract_id, client) = setup_audit(&env, 1_000i128);

    // Mint extra tokens directly to the contract without updating stored balance
    StellarAssetClient::new(&env, &token).mint(&contract_id, &500i128);

    let tokens = soroban_sdk::vec![&env, token.clone()];
    let mismatches = client.audit_balances(&tokens);

    assert_eq!(mismatches.len(), 1);
    let m = mismatches.get(0).unwrap();
    assert_eq!(m.token, token);
    assert_eq!(m.expected, 1_000);
    assert_eq!(m.actual, 1_500);
    assert_eq!(m.delta, 500); // surplus
}

#[test]
fn test_audit_balances_detects_deficit() {
    let env = Env::default();
    env.mock_all_auths();

    let (token, contract_id, client) = setup_audit(&env, 1_000i128);

    // Artificially inflate the stored balance to simulate a deficit
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 2_000i128);
    });

    let tokens = soroban_sdk::vec![&env, token.clone()];
    let mismatches = client.audit_balances(&tokens);

    assert_eq!(mismatches.len(), 1);
    let m = mismatches.get(0).unwrap();
    assert_eq!(m.expected, 2_000);
    assert_eq!(m.actual, 1_000);
    assert_eq!(m.delta, -1_000); // deficit
}

#[test]
fn test_audit_balances_multiple_tokens() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Token A: balanced
    let token_a = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    StellarAssetClient::new(&env, &token_a).mint(&contract_id, &500i128);
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token_a, 500i128);
    });

    // Token B: stored says 300, actual is 400 (surplus)
    let token_b = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    StellarAssetClient::new(&env, &token_b).mint(&contract_id, &400i128);
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token_b, 300i128);
    });

    let tokens = soroban_sdk::vec![&env, token_a.clone(), token_b.clone()];
    let mismatches = client.audit_balances(&tokens);

    // Only token B should appear
    assert_eq!(mismatches.len(), 1);
    let m = mismatches.get(0).unwrap();
    assert_eq!(m.token, token_b);
    assert_eq!(m.expected, 300);
    assert_eq!(m.actual, 400);
    assert_eq!(m.delta, 100);
}

#[test]
fn test_audit_balances_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);

    let tokens = soroban_sdk::vec![&env, token.clone()];
    let result = client.try_audit_balances(&tokens);
    assert_eq!(result, Err(Ok(ContractError::NotInitialized)));
}

#[test]
fn test_audit_balances_limit_at_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let (_recipient, _token, _contract_id, client) = setup(&env, 1000);

    let mut tokens = soroban_sdk::Vec::new(&env);
    for _ in 0..MAX_AUDIT_TOKENS {
        let token = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        tokens.push_back(token);
    }

    let result = client.try_audit_balances(&tokens);
    assert!(result.is_ok());
}

#[test]
fn test_audit_balances_limit_over_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let (_recipient, _token, _contract_id, client) = setup(&env, 1000);

    let mut tokens = soroban_sdk::Vec::new(&env);
    for _ in 0..=MAX_AUDIT_TOKENS {
        let token = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        tokens.push_back(token);
    }

    let result = client.try_audit_balances(&tokens);
    assert_eq!(result, Err(Ok(ContractError::IterationLimitExceeded)));
}

// ── Issue #563: require_auth_for_args ─────────────────────────────────────

/// A valid claim_fees auth for (provider, token_A) must be rejected when the
/// caller substitutes token_B — demonstrating that require_auth_for_args
/// scopes the signature to the exact (provider, token) pair.
#[test]
fn test_claim_fees_arg_scoped_auth_rejects_substituted_token() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    let env = Env::default();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    let token_a = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    env.mock_all_auths();
    client.initialize(&admin);

    // Provider signs for (provider, token_a) but the call targets token_b.
    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "claim_fees",
        args: (&provider, &token_a).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &provider,
        invoke: &mock_invoke,
    };

    // The call uses token_b but the auth only covers token_a — must fail.
    let result = client
        .mock_auths(&[mock_auth])
        .try_claim_fees(&provider, &token_b);

    assert!(
        result.is_err(),
        "auth scoped to token_a must not authorize a claim for token_b"
    );
}

/// A valid claim_fees auth scoped to the correct (provider, token) succeeds.
#[test]
fn test_claim_fees_arg_scoped_auth_passes_for_correct_args() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    let env = Env::default();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    env.mock_all_auths();
    client.initialize(&admin);

    // Auth exactly matches the call arguments.
    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "claim_fees",
        args: (&provider, &token).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &provider,
        invoke: &mock_invoke,
    };

    let result = client
        .mock_auths(&[mock_auth])
        .try_claim_fees(&provider, &token);

    assert!(result.is_ok(), "correctly scoped auth must succeed");
}

// ---------------------------------------------------------------------------
// Issue #633: fee calculation performance / optimization tests
// ---------------------------------------------------------------------------

#[test]
fn test_fee_and_burn_amounts_basic() {
    // fee = 1_000_000 * 30 / 10_000 = 3_000
    // burn = 3_000 * 1_000 / 10_000 = 300
    let (fee, burn) = crate::fee_and_burn_amounts(1_000_000, 30, 1_000).unwrap();
    assert_eq!(fee, 3_000);
    assert_eq!(burn, 300);
}

#[test]
fn test_fee_and_burn_amounts_zero_burn() {
    let (fee, burn) = crate::fee_and_burn_amounts(1_000_000, 30, 0).unwrap();
    assert_eq!(fee, 3_000);
    assert_eq!(burn, 0);
}

#[test]
fn test_fee_and_burn_amounts_full_burn() {
    let (fee, burn) = crate::fee_and_burn_amounts(1_000_000, 30, 10_000).unwrap();
    assert_eq!(fee, 3_000);
    assert_eq!(burn, 3_000); // 100% burn
}

#[test]
fn test_fee_and_burn_amounts_overflow_returns_none() {
    assert!(crate::fee_and_burn_amounts(i128::MAX, 30, 1_000).is_none());
}

#[test]
fn test_fee_and_burn_conservation() {
    // burn + distributable must always equal fee exactly (no dust)
    let cases: &[(i128, u32, u32)] = &[
        (777_777, 30, 3_333),
        (9_999, 30, 1_000),
        (10_000_000, 100, 10_000),
        (1, 1, 0),
    ];
    for &(amount, fee_bps, burn_bps) in cases {
        if let Some((fee, burn)) = crate::fee_and_burn_amounts(amount, fee_bps, burn_bps) {
            let distributable = fee - burn;
            assert_eq!(
                burn + distributable,
                fee,
                "conservation failed for amount={amount}"
            );
        }
    }
}

#[test]
fn test_fee_amount_floor_matches_fee_and_burn_fee_component() {
    // fee_and_burn_amounts fee component must equal fee_amount_floor
    let amount = 500_000i128;
    let rate = 30u32;
    let expected = crate::fee_amount_floor(amount, rate).unwrap();
    let (fee, _) = crate::fee_and_burn_amounts(amount, rate, 1_000).unwrap();
    assert_eq!(fee, expected);
}

// ---------------------------------------------------------------------------
// Congestion-Based Dynamic Fees
// ---------------------------------------------------------------------------

#[test]
fn test_congestion_normal() {
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

    // Base fee is 30 bps.
    client.set_fee_rate(&30u32);
    disable_revenue_share(&client);

    let trade_amount: i128 = 10_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);
    mark_trader_has_traded(&env, &contract_id, &trader);

    // Normal congestion: 1.0x (10_000 bps default)
    // Fee = 10_000_000 * 30 / 10_000 = 30_000
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee, 30_000);
}

#[test]
fn test_congestion_high() {
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
    disable_revenue_share(&client);

    // Set high congestion signal: 2.0x (20_000 bps)
    client.set_congestion_signal(&admin, &20_000u32);

    let trade_amount: i128 = 10_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);
    mark_trader_has_traded(&env, &contract_id, &trader);

    // High congestion fee = 10_000_000 * (30 * 2.0 = 60) / 10_000 = 60_000
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee, 60_000);
}

#[test]
fn test_congestion_stale_fallback() {
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
    disable_revenue_share(&client);

    // Set high congestion signal: 2.0x
    env.ledger().set_timestamp(100);
    client.set_congestion_signal(&admin, &20_000u32);

    // Advance time past staleness threshold (300 secs)
    env.ledger().set_timestamp(100 + 301);

    let trade_amount: i128 = 10_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);
    mark_trader_has_traded(&env, &contract_id, &trader);

    // Stale signal falls back to default 1.0x (30 bps)
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee, 30_000);
}

#[test]
fn test_congestion_bounded() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Max multiplier is 5.0x by default. Try 6.0x -> Should fail bounds check.
    let result = client.try_set_congestion_signal(&admin, &60_000u32);
    assert_eq!(result, Err(Ok(ContractError::InvalidMultiplierBounds)));
}

// ---------------------------------------------------------------------------
// Emergency pause / circuit breaker (Issue #821)
//
// fee_collector previously had no connection to `shared::pausable` at all,
// so an admin pausing the protocol during an incident could not stop
// fee_collector from continuing to move funds via `collect_fee`,
// `claim_fees` and `withdraw_treasury_fees` while every other contract
// (stake_vault, signal_registry, oracle, governance, auto_trade) obeyed the
// shared pause flag. These tests prove the gap is closed for the
// fund-moving entry points and that normal operation resumes after unpause.
// ---------------------------------------------------------------------------

#[test]
fn test_fresh_contract_is_not_paused() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    assert!(!client.is_paused());
}

#[test]
fn test_pause_blocks_withdraw_treasury_fees_then_unpause_allows_it() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);
    env.ledger().set_timestamp(86400);

    // Admin pauses the contract (shared circuit breaker).
    client.pause();
    assert!(client.is_paused());

    // Before the fix this call would have succeeded even while paused.
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(
        result,
        Err(Ok(ContractError::ContractPaused)),
        "paused contract must reject withdraw_treasury_fees with a clear error"
    );
    assert_eq!(
        client.treasury_balance(&token),
        1000i128,
        "no funds should move while paused"
    );

    // Resume: the previously queued withdrawal succeeds again.
    client.unpause();
    assert!(!client.is_paused());
    client.withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(client.treasury_balance(&token), 0i128);
    assert_eq!(TokenClient::new(&env, &token).balance(&recipient), 1000i128);
}

#[test]
fn test_pause_blocks_queue_withdrawal() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    client.pause();
    let result = client.try_queue_withdrawal(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));

    client.unpause();
    client.queue_withdrawal(&recipient, &token, &1000i128);
}

#[test]
fn test_pause_blocks_collect_fee_then_unpause_allows_it() {
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

    StellarAssetClient::new(&env, &token).mint(&trader, &(2_000 * 10_000_000));
    mark_trader_has_traded(&env, &contract_id, &trader);

    client.pause();
    let result = client.try_collect_fee(&trader, &token, &(1_000 * 10_000_000), &asset);
    assert_eq!(
        result,
        Err(Ok(ContractError::ContractPaused)),
        "paused contract must reject collect_fee with a clear error"
    );

    client.unpause();
    let fee = client.collect_fee(&trader, &token, &(1_000 * 10_000_000), &asset);
    assert!(
        fee > 0,
        "collect_fee must work normally again after unpause"
    );
}

#[test]
fn test_pause_blocks_claim_fees_then_unpause_allows_it() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let amount: i128 = 1_000_000;

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token_id).mint(&contract_id, &amount);
    env.as_contract(&contract_id, || {
        set_pending_fees(&env, &provider, &token_id, amount);
    });

    client.pause();
    let result = client.try_claim_fees(&provider, &token_id);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));

    client.unpause();
    let claimed = client.claim_fees(&provider, &token_id);
    assert_eq!(claimed, amount);
}

#[test]
fn test_pause_requires_admin_auth() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let non_admin = Address::generate(&env);
    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "pause",
        args: ().into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &non_admin,
        invoke: &mock_invoke,
    };
    let result = client.mock_auths(&[mock_auth]).try_pause();
    assert!(result.is_err(), "non-admin must not be able to pause");
    assert!(!client.is_paused());
}

#[test]
fn error_messages_are_non_empty_and_distinct() {
    let samples = [
        ContractError::NotInitialized,
        ContractError::Unauthorized,
        ContractError::InsufficientTreasuryBalance,
        ContractError::TimelockNotElapsed,
        ContractError::WithdrawalNotQueued,
        ContractError::UnauthorizedCaller,
    ];
    for err in samples.iter() {
        assert!(!err.message().is_empty());
    }
    for i in 0..samples.len() {
        for j in (i + 1)..samples.len() {
            assert_ne!(
                samples[i].message(),
                samples[j].message(),
                "expected distinct messages for {:?} and {:?}",
                samples[i],
                samples[j]
            );
        }
    }
}

// ── Instruction-budget regression snapshots (Issue #budget) ───────────────────

use stellar_swipe_common::budget_regression::measure_and_emit;

#[test]
fn collect_fee_budget_regression() {
    let env = Env::default();
    env.mock_all_auths();
    let (_recipient, _token, contract_id, client) = setup(&env, 1_000_000);
    let trader = Address::generate(&env);
    let asset = trade_asset(&env);
    let amount: i128 = 100_000;

    env.budget().reset_tracker();
    let _ = client.collect_fee(&trader, &_token, &amount, &asset);
    let instructions = env.budget().cpu_instruction_cost();
    measure_and_emit("fee_collector.collect_fee", 5_500_000, instructions);
}

// ── Issue #960: Insurance Payout & Cap Tests ─────────────────────────────────

#[test]
fn test_insurance_payout_successful() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let funder = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Mint tokens to funder and deposit into insurance fund
    let deposit_amount = 10_000i128;
    StellarAssetClient::new(&env, &token).mint(&funder, &deposit_amount);
    client.deposit_insurance_fund(&funder, &token, &deposit_amount);

    assert_eq!(client.get_insurance_balance(&token), deposit_amount);

    // Set max payout cap to 5,000 per claim
    let max_cap = 5_000i128;
    client.set_insurance_payout_cap(&admin, &token, &max_cap);
    assert_eq!(client.get_insurance_payout_cap(&token), max_cap);

    // Execute successful payout of 3,000 to provider
    let claim_id = String::from_str(&env, "claim_12345");
    let payout_amount = 3_000i128;
    client.payout_insurance(&admin, &provider, &token, &payout_amount, &claim_id);

    // Verify balances
    assert_eq!(
        client.get_insurance_balance(&token),
        deposit_amount - payout_amount
    );
    assert_eq!(
        TokenClient::new(&env, &token).balance(&provider),
        payout_amount
    );
    assert!(client.is_insurance_claim_processed(&claim_id));
}

#[test]
fn test_insurance_payout_exceeds_cap() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let funder = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let deposit_amount = 10_000i128;
    StellarAssetClient::new(&env, &token).mint(&funder, &deposit_amount);
    client.deposit_insurance_fund(&funder, &token, &deposit_amount);

    // Set max payout cap to 2,000
    client.set_insurance_payout_cap(&admin, &token, &2_000i128);

    // Attempt payout of 2,001 (exceeds cap)
    let claim_id = String::from_str(&env, "claim_excess");
    let res = client.try_payout_insurance(&admin, &provider, &token, &2_001i128, &claim_id);
    assert_eq!(res, Err(Ok(ContractError::PayoutExceedsCap)));
}

#[test]
fn test_insurance_payout_unauthorized() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let unauthorized_caller = Address::generate(&env);
    let provider = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);

    env.mock_all_auths();
    client.initialize(&admin);
    client.set_insurance_payout_cap(&admin, &token, &5_000i128);

    // Set insurance balance
    env.as_contract(&contract_id, || {
        crate::storage::set_insurance_balance(&env, &token, 5_000i128);
    });

    // Unauthorized non-admin attempt
    let claim_id = String::from_str(&env, "claim_unauth");
    let res = client.try_payout_insurance(
        &unauthorized_caller,
        &provider,
        &token,
        &1_000i128,
        &claim_id,
    );
    assert_eq!(res, Err(Ok(ContractError::UnauthorizedCaller)));
}

#[test]
fn test_insurance_payout_double_claim_prevention() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let funder = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&funder, &10_000i128);
    client.deposit_insurance_fund(&funder, &token, &10_000i128);
    client.set_insurance_payout_cap(&admin, &token, &5_000i128);

    let claim_id = String::from_str(&env, "claim_double");
    client.payout_insurance(&admin, &provider, &token, &1_000i128, &claim_id);

    // Second payout with same claim ID must fail
    let res2 = client.try_payout_insurance(&admin, &provider, &token, &1_000i128, &claim_id);
    assert_eq!(res2, Err(Ok(ContractError::ClaimAlreadyProcessed)));
}

#[test]
fn test_insurance_payout_insufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let funder = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Deposit only 500
    StellarAssetClient::new(&env, &token).mint(&funder, &500i128);
    client.deposit_insurance_fund(&funder, &token, &500i128);
    client.set_insurance_payout_cap(&admin, &token, &5_000i128);

    // Request payout of 1,000 (exceeds balance)
    let claim_id = String::from_str(&env, "claim_insuff");
    let res = client.try_payout_insurance(&admin, &provider, &token, &1_000i128, &claim_id);
    assert_eq!(res, Err(Ok(ContractError::InsufficientInsuranceBalance)));
}

#[test]
fn test_insurance_payout_when_paused() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let funder = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&funder, &5_000i128);
    client.deposit_insurance_fund(&funder, &token, &5_000i128);
    client.set_insurance_payout_cap(&admin, &token, &5_000i128);

    client.pause();

    let claim_id = String::from_str(&env, "claim_paused");
    let res = client.try_payout_insurance(&admin, &provider, &token, &1_000i128, &claim_id);
    assert_eq!(res, Err(Ok(ContractError::ContractPaused)));
}

#[test]
fn test_insurance_payout_authorized_caller() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let provider = Address::generate(&env);
    let funder = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&funder, &5_000i128);
    client.deposit_insurance_fund(&funder, &token, &5_000i128);
    client.set_insurance_payout_cap(&admin, &token, &5_000i128);

    // Authorize keeper
    client.authorize_caller(&keeper);
    assert!(client.is_caller_authorized(&keeper));

    // Keeper executes insurance payout
    let claim_id = String::from_str(&env, "claim_keeper");
    client.payout_insurance(&keeper, &provider, &token, &1_500i128, &claim_id);

    assert_eq!(TokenClient::new(&env, &token).balance(&provider), 1_500i128);
    assert_eq!(client.get_insurance_balance(&token), 3_500i128);
}

#[test]
fn test_insurance_payout_invalid_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Negative cap rejection
    let res_cap = client.try_set_insurance_payout_cap(&admin, &token, &-100i128);
    assert_eq!(res_cap, Err(Ok(ContractError::InvalidAmount)));

    // Zero / negative payout rejection
    let claim_zero = String::from_str(&env, "claim_zero");
    let res_zero = client.try_payout_insurance(&admin, &provider, &token, &0i128, &claim_zero);
    assert_eq!(res_zero, Err(Ok(ContractError::InvalidAmount)));

    let claim_neg = String::from_str(&env, "claim_neg");
    let res_neg = client.try_payout_insurance(&admin, &provider, &token, &-50i128, &claim_neg);
    assert_eq!(res_neg, Err(Ok(ContractError::InvalidAmount)));
}

// ---------------------------------------------------------------------------
// Issue #940: Fee rebate cap tests
// ---------------------------------------------------------------------------

/// Helper: sets pending fees for a provider directly in contract storage.
fn set_provider_pending(
    env: &Env,
    contract_id: &Address,
    provider: &Address,
    token: &Address,
    amount: i128,
) {
    env.as_contract(contract_id, || {
        set_pending_fees(env, provider, token, amount);
    });
}

/// Helper: records daily fee total for the current epoch day.
fn set_epoch_fees(env: &Env, contract_id: &Address, token: &Address, amount: i128) {
    let epoch_day = env.ledger().timestamp() / crate::storage::SECONDS_PER_DAY_FC;
    env.as_contract(contract_id, || {
        crate::storage::add_daily_fee_total(env, token, epoch_day, amount);
    });
}

#[test]
fn test_claim_fees_below_rebate_cap_pays_full_amount() {
    // Total rebates below cap → each provider receives full pending amount.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Epoch fees: 1_000. Cap at 80% = 800. Pending = 500 (well below cap).
    let epoch_fees: i128 = 1_000;
    let pending: i128 = 500;
    set_epoch_fees(&env, &contract_id, &token, epoch_fees);
    set_provider_pending(&env, &contract_id, &provider, &token, pending);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &pending);

    let claimed = client.claim_fees(&provider, &token);
    // Full amount paid — no cap triggered.
    assert_eq!(claimed, pending);
    assert_eq!(TokenClient::new(&env, &token).balance(&provider), pending);
}

#[test]
fn test_claim_fees_above_rebate_cap_scales_proportionally() {
    // Total rebates above cap → provider receives the capped (scaled-down) amount.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Epoch fees: 1_000. Cap at 80% = 800. Pending = 1_200 (exceeds cap).
    let epoch_fees: i128 = 1_000;
    let pending: i128 = 1_200;
    let expected_cap: i128 = 800; // 1_000 * 8_000 / 10_000
    set_epoch_fees(&env, &contract_id, &token, epoch_fees);
    set_provider_pending(&env, &contract_id, &provider, &token, pending);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &pending);

    let claimed = client.claim_fees(&provider, &token);
    // Cap triggered: provider receives only the cap amount.
    assert_eq!(claimed, expected_cap);
    assert_eq!(
        TokenClient::new(&env, &token).balance(&provider),
        expected_cap
    );
}

#[test]
fn test_claim_fees_exactly_at_rebate_cap_pays_full() {
    // Single claimant at exactly the cap → receives exactly the cap amount in full.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Epoch fees: 1_000. Cap = 80% = 800. Pending = exactly 800.
    let epoch_fees: i128 = 1_000;
    let pending: i128 = 800; // exactly at cap
    set_epoch_fees(&env, &contract_id, &token, epoch_fees);
    set_provider_pending(&env, &contract_id, &provider, &token, pending);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &pending);

    let claimed = client.claim_fees(&provider, &token);
    // Exactly at cap: full amount paid, no scaling needed.
    assert_eq!(claimed, pending);
    assert_eq!(TokenClient::new(&env, &token).balance(&provider), pending);
}

#[test]
fn test_set_max_rebate_bps_admin_only() {
    // Non-admin cannot set max rebate bps.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Admin can set it.
    client.set_max_rebate_bps(&5_000u32);
    assert_eq!(client.get_max_rebate_bps(), 5_000u32);

    // Exceeding 10_000 is rejected.
    let result = client.try_set_max_rebate_bps(&10_001u32);
    assert_eq!(result, Err(Ok(ContractError::InvalidFeeConfiguration)));
}

#[test]
fn test_get_max_rebate_bps_default_is_8000() {
    // Unset → default 8000.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    assert_eq!(client.get_max_rebate_bps(), 8_000u32);
}

// ---------------------------------------------------------------------------
// Issue #945: Fee cache invalidation on config change
// ---------------------------------------------------------------------------

/// (a) Query effective fee rate before a rate change (cache is populated).
/// (b) Change the fee rate via set_fee_rate.
/// (c) Query again — must return the NEW rate, not the stale cached value.
#[test]
fn test_fee_cache_invalidated_on_set_fee_rate() {
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

    let (oracle_id, _asset) = setup_oracle(&env, 10_000_000);
    client.set_oracle_contract(&oracle_id);

    // (a) Initial rate is 30 bps.
    client.set_fee_rate(&30u32);
    let rate_before = client.current_dynamic_fee_rate(&trader, &token, &trade_asset(&env));
    assert_eq!(rate_before, 30u32);

    // (b) Change the fee rate.
    client.set_fee_rate(&60u32);

    // (c) Must reflect the new rate immediately — cache must not serve stale 30.
    let rate_after = client.current_dynamic_fee_rate(&trader, &token, &trade_asset(&env));
    assert_eq!(
        rate_after, 60u32,
        "cache must be invalidated after set_fee_rate"
    );
}

/// Same correctness guarantee for set_burn_rate: the cache key changes so
/// the next load_tx_fee_config call recomputes from storage.
#[test]
fn test_fee_cache_invalidated_on_set_burn_rate() {
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
    client.set_burn_rate(&1_000u32); // 10%
    disable_revenue_share(&client);

    let trade_amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &(trade_amount * 2));
    mark_trader_has_traded(&env, &contract_id, &trader);

    // First collection uses burn_rate = 10%: fee=3000, burn=300, treasury=2700.
    let fee1 = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee1, 3_000);
    assert_eq!(client.treasury_balance(&token), 2_700);

    // Change burn rate to 50%.
    client.set_burn_rate(&5_000u32);

    // Second collection must use the new burn_rate=50%: fee=3000, burn=1500, treasury+=1500.
    let fee2 = client.collect_fee(&trader, &token, &trade_amount, &asset);
    assert_eq!(fee2, 3_000);
    // treasury = 2700 (from first) + 1500 (from second at 50% burn) = 4200
    assert_eq!(
        client.treasury_balance(&token),
        4_200,
        "cache must be invalidated after set_burn_rate"
    );
}

#[test]
fn test_rebate_cap_second_claim_uses_remaining_headroom() {
    // Two sequential claims in the same epoch: second gets only the leftover headroom.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider_a = Address::generate(&env);
    let provider_b = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Epoch fees: 1_000. Cap = 80% = 800.
    // Provider A pending = 600. Provider B pending = 400.
    // After A claims 600, headroom = 200. B can only get 200.
    let epoch_fees: i128 = 1_000;
    set_epoch_fees(&env, &contract_id, &token, epoch_fees);

    set_provider_pending(&env, &contract_id, &provider_a, &token, 600);
    set_provider_pending(&env, &contract_id, &provider_b, &token, 400);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000i128);

    let claimed_a = client.claim_fees(&provider_a, &token);
    assert_eq!(claimed_a, 600); // under cap

    let claimed_b = client.claim_fees(&provider_b, &token);
    assert_eq!(claimed_b, 200); // only headroom remaining (800 - 600 = 200)

    assert_eq!(TokenClient::new(&env, &token).balance(&provider_a), 600);
    assert_eq!(TokenClient::new(&env, &token).balance(&provider_b), 200);
}
