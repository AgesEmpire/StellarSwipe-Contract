#![cfg(test)]

use soroban_sdk::{testutils::Address as _, vec, Address, Env};

use crate::{
    events::SCHEMA_VERSION,
    set_treasury_balance,
    volatility::{VolatilityBand, VolatilityBandConfig},
    ContractError, FeeCollector, FeeCollectorClient, MAX_AUDIT_TOKENS,
};

fn setup(env: &Env) -> (Address, Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &id);
    client.initialize(&admin);
    (admin, id, client)
}

fn make_token(env: &Env) -> Address {
    env.register_stellar_asset_contract_v2(Address::generate(env))
        .address()
}

// ── Issue 1: Volatility-based fee bands ──────────────────────────────────────

#[test]
fn test_volatility_bands_base_rate_when_no_signal() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let bands = VolatilityBandConfig {
        base_fee_rate_bps: 20,
        bands: vec![
            &env,
            VolatilityBand {
                volatility_threshold_bps: 20_000,
                fee_rate_bps: 50,
            },
            VolatilityBand {
                volatility_threshold_bps: 30_000,
                fee_rate_bps: 80,
            },
        ],
    };
    client.set_volatility_bands(&bands);

    // No congestion signal set → base rate applies
    assert_eq!(client.current_volatility_fee_rate(), 20u32);
}

#[test]
fn test_volatility_bands_selects_correct_band_at_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let bands = VolatilityBandConfig {
        base_fee_rate_bps: 10,
        bands: vec![
            &env,
            VolatilityBand {
                volatility_threshold_bps: 15_000,
                fee_rate_bps: 30,
            },
            VolatilityBand {
                volatility_threshold_bps: 25_000,
                fee_rate_bps: 60,
            },
        ],
    };
    client.set_volatility_bands(&bands);

    // Set signal exactly at first threshold
    client.set_congestion_signal(&admin, &15_000u32);
    assert_eq!(client.current_volatility_fee_rate(), 30u32);

    // Set signal at second threshold
    client.set_congestion_signal(&admin, &25_000u32);
    assert_eq!(client.current_volatility_fee_rate(), 60u32);
}

#[test]
fn test_volatility_bands_below_all_thresholds_uses_base() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let bands = VolatilityBandConfig {
        base_fee_rate_bps: 5,
        bands: vec![
            &env,
            VolatilityBand {
                volatility_threshold_bps: 20_000,
                fee_rate_bps: 40,
            },
        ],
    };
    client.set_volatility_bands(&bands);

    // Signal below all thresholds → base rate
    client.set_congestion_signal(&admin, &10_000u32);
    assert_eq!(client.current_volatility_fee_rate(), 5u32);
}

#[test]
fn test_volatility_bands_extreme_high_volatility() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let bands = VolatilityBandConfig {
        base_fee_rate_bps: 10,
        bands: vec![
            &env,
            VolatilityBand {
                volatility_threshold_bps: 15_000,
                fee_rate_bps: 30,
            },
            VolatilityBand {
                volatility_threshold_bps: 40_000,
                fee_rate_bps: 100,
            },
        ],
    };
    client.set_volatility_bands(&bands);

    // Max allowed multiplier (50_000 bps = 5x)
    client.set_congestion_signal(&admin, &50_000u32);
    // Both thresholds met; highest matching band (40_000) wins
    assert_eq!(client.current_volatility_fee_rate(), 100u32);
}

#[test]
fn test_set_volatility_bands_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let bands = VolatilityBandConfig {
        base_fee_rate_bps: 10,
        bands: vec![&env],
    };

    env.set_auths(&[]);
    let result = client.try_set_volatility_bands(&bands);
    assert!(result.is_err());
}

#[test]
fn test_get_volatility_band_config_none_before_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);
    assert!(client.get_volatility_band_config().is_none());
}

// ── Issue 3: Batch snapshot ───────────────────────────────────────────────────

#[test]
fn test_batch_snapshot_empty_tokens() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let snapshot = client.batch_snapshot(&soroban_sdk::Vec::new(&env));
    assert_eq!(snapshot.total_amount, 0);
    assert_eq!(snapshot.entries.len(), 0);
}

#[test]
fn test_batch_snapshot_returns_pool_amounts() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, contract_id, client) = setup(&env);

    let token_a = make_token(&env);
    let token_b = make_token(&env);

    env.as_contract(&contract_id, || {
        crate::storage::add_revenue_share_pool(&env, &token_a, 1_000);
        crate::storage::add_revenue_share_pool(&env, &token_b, 2_500);
    });

    let tokens = vec![&env, token_a.clone(), token_b.clone()];
    let snapshot = client.batch_snapshot(&tokens);

    assert_eq!(snapshot.total_amount, 3_500);
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries.get(0).unwrap().amount, 1_000);
    assert_eq!(snapshot.entries.get(1).unwrap().amount, 2_500);
}

#[test]
fn test_batch_snapshot_does_not_mutate_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, contract_id, client) = setup(&env);

    let token = make_token(&env);
    env.as_contract(&contract_id, || {
        crate::storage::add_revenue_share_pool(&env, &token, 500);
    });

    let tokens = vec![&env, token.clone()];
    let before = client.batch_snapshot(&tokens).total_amount;
    let after = client.batch_snapshot(&tokens).total_amount;

    assert_eq!(before, after);
    assert_eq!(before, 500);
}

#[test]
fn test_batch_snapshot_exceeds_limit_returns_error() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let mut tokens = soroban_sdk::Vec::new(&env);
    for _ in 0..=MAX_AUDIT_TOKENS {
        tokens.push_back(make_token(&env));
    }

    let result = client.try_batch_snapshot(&tokens);
    assert_eq!(result, Err(Ok(ContractError::IterationLimitExceeded)));
}

#[test]
fn test_batch_snapshot_not_initialized_returns_error() {
    let env = Env::default();
    env.mock_all_auths();

    let id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &id);

    let result = client.try_batch_snapshot(&soroban_sdk::Vec::new(&env));
    assert_eq!(result, Err(Ok(ContractError::NotInitialized)));
}

// ── Issue 4: Event schema versioning ─────────────────────────────────────────

#[test]
fn test_event_schema_version_is_one() {
    assert_eq!(SCHEMA_VERSION, 1u32);
}

#[test]
fn test_fee_rate_updated_event_emitted() {
    use soroban_sdk::testutils::Events;

    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    client.set_fee_rate(&30u32);

    let events = env.events().all();
    assert!(!events.is_empty(), "FeeRateUpdated event must be emitted");
}

#[test]
fn test_snapshot_recorded_event_struct_fields_stable() {
    // Verify SnapshotRecorded contractevent struct shape is stable for indexers.
    use crate::events::SnapshotRecorded;

    let evt = SnapshotRecorded {
        ledger: 1,
        timestamp: 1000,
        total_amount: 500,
        entry_count: 2,
    };
    assert_eq!(evt.ledger, 1u64);
    assert_eq!(evt.entry_count, 2u32);
    assert_eq!(evt.total_amount, 500i128);
    assert_eq!(evt.timestamp, 1000u64);
}

#[test]
fn test_fees_claimed_event_fields_backward_compatible() {
    use crate::events::FeesClaimed;
    let env = Env::default();
    env.mock_all_auths();

    let provider = Address::generate(&env);
    let token = make_token(&env);
    let evt = FeesClaimed {
        provider: provider.clone(),
        token: token.clone(),
        amount: 100,
    };
    assert_eq!(evt.amount, 100i128);
    assert_eq!(evt.provider, provider);
    assert_eq!(evt.token, token);
}

#[test]
fn test_fee_rate_updated_event_fields_backward_compatible() {
    use crate::events::FeeRateUpdated;
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let evt = FeeRateUpdated {
        old_rate: 20,
        new_rate: 30,
        updated_by: admin,
    };
    assert_eq!(evt.old_rate, 20u32);
    assert_eq!(evt.new_rate, 30u32);
}

#[test]
fn test_withdrawal_queued_event_fields_backward_compatible() {
    use crate::events::WithdrawalQueued;
    let env = Env::default();
    env.mock_all_auths();

    let recipient = Address::generate(&env);
    let token = make_token(&env);
    let evt = WithdrawalQueued {
        recipient: recipient.clone(),
        token: token.clone(),
        amount: 500,
        available_at: 86400,
    };
    assert_eq!(evt.amount, 500i128);
    assert_eq!(evt.available_at, 86400u64);
}
