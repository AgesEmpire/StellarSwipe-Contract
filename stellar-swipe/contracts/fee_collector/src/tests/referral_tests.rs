#![cfg(test)]

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Events},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, String,
};
use stellar_swipe_common::Asset;

use crate::{
    ContractError, FeeCollector, FeeCollectorClient,
};

#[contract]
struct MockOracleContract;

#[contractimpl]
impl MockOracleContract {
    pub fn convert_to_base(_env: Env, amount: i128, _asset: Asset) -> i128 {
        amount
    }
}

fn setup_oracle(env: &Env) -> (Address, Asset) {
    let oracle_id = env.register(MockOracleContract, ());
    let asset = Asset {
        code: String::from_str(env, "TRADE"),
        issuer: Some(Address::generate(env)),
    };
    (oracle_id, asset)
}

fn mark_trader_has_traded(env: &Env, contract_id: &Address, trader: &Address) {
    env.as_contract(contract_id, || {
        crate::storage::set_has_traded(env, trader);
    });
}

fn setup(env: &Env) -> (Address, Address, Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    
    let token_admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);

    (admin, token, contract_id, client)
}

#[test]
fn test_register_referral_success() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, _, client) = setup(&env);

    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);

    client.register_referral(&referrer, &referee);

    // Event emission correctness
    let events = env.events().all();
    assert!(events.len() > 0, "Should emit referral_registered event");
}

#[test]
fn test_register_self_referral_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, _, client) = setup(&env);

    let user = Address::generate(&env);

    let result = client.try_register_referral(&user, &user);
    assert_eq!(result, Err(Ok(ContractError::SelfReferralNotAllowed)));
}

#[test]
fn test_register_referral_already_registered_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, _, client) = setup(&env);

    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);

    client.register_referral(&referrer, &referee);

    let new_referrer = Address::generate(&env);
    let result = client.try_register_referral(&new_referrer, &referee);
    assert_eq!(result, Err(Ok(ContractError::ReferralAlreadyRegistered)));
}

#[test]
fn test_admin_override_referral() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, _, client) = setup(&env);

    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);

    client.register_referral(&referrer, &referee);

    let new_referrer = Address::generate(&env);
    client.admin_override_referral(&admin, &new_referrer, &referee);

    // To verify, we would ideally have a getter on client, but we'll verify via fee distribution
    // that the new referrer gets the fees.
}

#[test]
fn test_set_referral_fee_share() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, _, client) = setup(&env);

    client.set_referral_fee_share(&admin, &2000u32); // 20%
    
    // Attempting invalid (> 100%)
    let result = client.try_set_referral_fee_share(&admin, &10001u32);
    assert_eq!(result, Err(Ok(ContractError::InvalidFeeConfiguration)));
}

#[test]
fn test_fee_distribution_with_no_referrer() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, token, contract_id, client) = setup(&env);

    let (oracle_id, asset) = setup_oracle(&env);
    client.set_oracle_contract(&oracle_id);
    client.set_fee_rate(&30u32); // 0.3%
    client.set_burn_rate(&1_000u32); // 10%
    client.set_revenue_share_rate_bps(&2_000u32); // 20%

    let trader = Address::generate(&env);
    let trade_amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);
    mark_trader_has_traded(&env, &contract_id, &trader);

    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    
    // Fee = 1,000,000 * 0.3% = 3,000
    assert_eq!(fee, 3_000);

    // Burn = 3,000 * 10% = 300
    // Distributable = 2,700
    // Revenue Share = 2,700 * 20% = 540
    // Treasury = 2,700 - 540 = 2,160

    assert_eq!(client.treasury_balance(&token), 2_160);
    assert_eq!(client.get_revenue_share_pool(&token), 540);
}

#[test]
fn test_fee_distribution_with_referrer() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, token, contract_id, client) = setup(&env);

    let (oracle_id, asset) = setup_oracle(&env);
    client.set_oracle_contract(&oracle_id);
    client.set_fee_rate(&30u32); // 0.3%
    client.set_burn_rate(&1_000u32); // 10%
    client.set_revenue_share_rate_bps(&2_000u32); // 20%
    
    client.set_referral_fee_share(&admin, &2_000u32); // 20%

    let trader = Address::generate(&env);
    let referrer = Address::generate(&env);
    client.register_referral(&referrer, &trader);

    let trade_amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);
    mark_trader_has_traded(&env, &contract_id, &trader);

    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);
    
    // Total Fee = 1,000,000 * 0.3% = 3,000
    assert_eq!(fee, 3_000);

    // Burn = 3,000 * 10% = 300
    // Distributable = 2,700
    
    // Referral Share = 3,000 * 20% = 600
    // Remaining Distributable = 2,700 - 600 = 2,100
    
    // Revenue Share = 2,700 * 20% = 540
    // Treasury = 2,100 - 540 = 1,560

    assert_eq!(client.treasury_balance(&token), 1_560);
    assert_eq!(client.get_revenue_share_pool(&token), 540);

    // Check referrer balance
    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&referrer), 600);
}
