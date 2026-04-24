#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, Bytes, Env, String};

#[test]
fn test_import_csv_valid_signals() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(
        &env,
        b"asset_pair,action,price,rationale,expiry_hours\nXLM/USDC,BUY,120000,Technical breakout,24\nBTC/USDC,SELL,45000000,Overbought RSI,48\nETH/USDC,BUY,3000000,Support level,36"
    );

    let result = client.import_signals_csv(&provider, &csv_data, &false);

    assert_eq!(result.success_count, 3);
    assert_eq!(result.error_count, 0);
}

#[test]
fn test_import_csv_validate_only() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(
        &env,
        b"asset_pair,action,price,rationale,expiry_hours\nXLM/USDC,BUY,120000,Test signal,24",
    );

    let result = client.import_signals_csv(&provider, &csv_data, &true);

    // In validate_only mode, success_count is 0
    assert_eq!(result.success_count, 0);
    assert_eq!(result.error_count, 0);
}

#[test]
fn test_import_csv_with_invalid_signal() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(
        &env,
        b"asset_pair,action,price,rationale,expiry_hours\nXLM/USDC,BUY,120000,Valid signal,24\nBTC/USDC,SELL,-45000,Invalid negative price,48\nETH/USDC,BUY,3000000,Another valid,36"
    );

    let result = client.import_signals_csv(&provider, &csv_data, &false);

    assert_eq!(result.success_count, 2);
    assert_eq!(result.error_count, 1);
}

#[test]
fn test_import_csv_empty_data() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(&env, b"");

    let result = client.import_signals_csv(&provider, &csv_data, &false);

    assert_eq!(result.success_count, 0);
    assert_eq!(result.error_count, 1);
}

#[test]
fn test_import_csv_invalid_format() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(
        &env,
        b"asset_pair,action,price,rationale,expiry_hours\nXLM/USDC,BUY,120000",
    );

    let result = client.import_signals_csv(&provider, &csv_data, &false);

    assert_eq!(result.error_count, 1);
}

#[test]
fn test_import_invalid_action() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(
        &env,
        b"asset_pair,action,price,rationale,expiry_hours\nXLM/USDC,HOLD,120000,Invalid action,24",
    );

    let result = client.import_signals_csv(&provider, &csv_data, &false);

    assert_eq!(result.error_count, 1);
}

#[test]
fn test_import_invalid_asset_pair() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(
        &env,
        b"asset_pair,action,price,rationale,expiry_hours\nXLMUSDC,BUY,120000,Missing slash,24",
    );

    let result = client.import_signals_csv(&provider, &csv_data, &false);

    assert_eq!(result.error_count, 1);
}

#[test]
fn test_import_mixed_valid_invalid() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let csv_data = Bytes::from_slice(
        &env,
        b"asset_pair,action,price,rationale,expiry_hours\nXLM/USDC,BUY,120000,Valid 1,24\nBTC/USDC,INVALID,45000,Invalid action,48\nETH/USDC,SELL,3000000,Valid 2,36\nDOT/USDC,BUY,-1000,Invalid price,24\nADA/USDC,BUY,500000,Valid 3,48"
    );

    let result = client.import_signals_csv(&provider, &csv_data, &false);

    assert_eq!(result.success_count, 3);
    assert_eq!(result.error_count, 2);
}

#[test]
fn test_external_id_mapping() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    client.initialize(&admin);

    let external_id = String::from_str(&env, "EXT123");
    let result = client.get_signal_by_external_id(&provider, &external_id);

    // Should return None since no mapping exists
    assert!(result.is_none());
}
