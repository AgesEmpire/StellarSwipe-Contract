#![cfg(test)]

//! Error taxonomy coverage (Issue #1033).
//!
//! Verifies that representative contract failures surface **stable numeric
//! codes** and that each maps to exactly one `shared::errors::ErrorCategory`,
//! so off-chain clients can branch on the *kind* of failure without hard-coding
//! per-contract numbers. See `docs/error_taxonomy.md`.

use shared::errors::{ErrorCategory, RecoveryStrategy};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    Address, Env,
};
use stellar_swipe_common::{
    collateral_oracle::{self, CollateralError},
    join_rate_limit::{self, JoinRateLimitConfig, JoinRateLimitError},
    oracle::{OraclePrice, MAX_PRICE_AGE_SECS},
};

use crate::{ContractError, FeeCollector, FeeCollectorClient};

/// Canonical mapping for the `fee_collector` error codes exercised below.
fn fee_error_category(code: u32) -> ErrorCategory {
    match code {
        3 => ErrorCategory::Authorization,      // Unauthorized
        4 => ErrorCategory::Validation,         // InvalidAmount
        5 => ErrorCategory::InvariantViolation, // InsufficientTreasuryBalance
        16 => ErrorCategory::Validation,        // InvalidFeeConfiguration
        28 => ErrorCategory::Authorization,     // UnauthorizedCaller
        _ => panic!("unmapped fee_collector error code {code}"),
    }
}

fn setup(env: &Env) -> (Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);
    (admin, client)
}

#[test]
fn invalid_input_failures_are_validation_category() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);

    // Fee split shares that do not sum to 100%.
    let err = client
        .try_set_fee_split_policy(&4_000, &4_000)
        .err()
        .unwrap()
        .unwrap();
    assert_eq!(err, ContractError::InvalidFeeConfiguration);
    assert_eq!(err as u32, 16);
    assert_eq!(fee_error_category(16), ErrorCategory::Validation);
    assert!(!ErrorCategory::Validation.is_transient());
    assert_eq!(
        ErrorCategory::Validation.default_strategy(),
        RecoveryStrategy::Escalate
    );
}

#[test]
fn unauthorized_action_failures_are_authorization_category() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let stranger = Address::generate(&env);
    let provider = Address::generate(&env);

    let err = client
        .try_record_provider_gross_fee_share(&stranger, &provider, &1_000)
        .err()
        .unwrap()
        .unwrap();
    assert_eq!(err, ContractError::UnauthorizedCaller);
    assert_eq!(err as u32, 28);
    assert_eq!(fee_error_category(28), ErrorCategory::Authorization);
}

#[test]
fn invariant_break_failures_are_invariant_category() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, client) = setup(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    // Withdrawing more than the treasury holds would break fund conservation.
    let err = client
        .try_queue_withdrawal(&recipient, &token, &1_000)
        .err()
        .unwrap()
        .unwrap();
    assert_eq!(err, ContractError::InsufficientTreasuryBalance);
    assert_eq!(err as u32, 5);
    assert_eq!(fee_error_category(5), ErrorCategory::InvariantViolation);
    assert_eq!(
        ErrorCategory::InvariantViolation.default_strategy(),
        RecoveryStrategy::ManualReview
    );
}

#[test]
fn capacity_limit_failures_are_capacity_category() {
    let env = Env::default();
    let contract_id = env.register(FeeCollector, ());

    env.as_contract(&contract_id, || {
        join_rate_limit::set_config(
            &env,
            JoinRateLimitConfig {
                window_secs: 3_600,
                max_joins_per_window: 2,
            },
        );
        let asset = symbol_short!("XLM");
        join_rate_limit::try_consume(&env, &asset).unwrap();
        join_rate_limit::try_consume(&env, &asset).unwrap();

        let err = join_rate_limit::check(&env, &asset).unwrap_err();
        assert_eq!(err, JoinRateLimitError::Exceeded);
        assert_eq!(err as u32, 1);

        // Documented category for rate-limit rejections.
        let category = ErrorCategory::CapacityLimit;
        assert!(category.is_transient());
        assert_eq!(category.default_strategy(), RecoveryStrategy::Defer);
        assert_eq!(category.slug(), "capacity_limit");
    });
}

#[test]
fn stale_oracle_failures_are_external_dependency_category() {
    let env = Env::default();
    let contract_id = env.register(FeeCollector, ());

    env.as_contract(&contract_id, || {
        env.ledger().set_timestamp(1_000_000);
        let now = env.ledger().timestamp();
        let stale = OraclePrice {
            price: 1_000_000,
            decimals: 6,
            timestamp: now - MAX_PRICE_AGE_SECS - 1,
            source: symbol_short!("band"),
        };

        let err = collateral_oracle::evaluate_collateral_health(
            &env,
            &stale,
            1_000,
            500,
            15_000,
            MAX_PRICE_AGE_SECS,
        )
        .unwrap_err();
        assert_eq!(err, CollateralError::PriceStale);
        assert_eq!(err as u32, 1);

        let category = ErrorCategory::ExternalDependency;
        assert!(category.is_transient());
        assert_eq!(category.slug(), "external_dependency");
    });
}

#[test]
fn every_taxonomy_category_has_a_distinct_stable_code() {
    let all = [
        (ErrorCategory::Validation, 1u32, "validation"),
        (ErrorCategory::Authorization, 2, "authorization"),
        (ErrorCategory::ExternalDependency, 3, "external_dependency"),
        (ErrorCategory::Arithmetic, 4, "arithmetic"),
        (ErrorCategory::Upgrade, 5, "upgrade"),
        (ErrorCategory::Network, 6, "network"),
        (ErrorCategory::Recovery, 7, "recovery"),
        (ErrorCategory::CapacityLimit, 8, "capacity_limit"),
        (ErrorCategory::InvariantViolation, 9, "invariant_violation"),
    ];
    for (cat, code, slug) in all {
        assert_eq!(cat as u32, code);
        assert_eq!(cat.slug(), slug);
    }
}
