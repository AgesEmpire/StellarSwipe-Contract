//! Public error ABI compatibility for `trade_executor` (Issue #1338).
//!
//! `error-abi/public-error-abi.json` pins the numeric code of every public
//! `#[contracterror]` variant; `scripts/check_error_abi.py` enforces it
//! against the source in CI (see `signal_registry/tests/error_abi.rs` for
//! the original, which this mirrors for `trade_executor::ContractError`).
//! Until now `trade_executor` had no compiled-crate test proving the
//! generated client actually surfaces the pinned numeric code — only the
//! Python/source-level check did. These tests close that gap: they assert
//! `ContractError`'s discriminants match the committed fixture, that the
//! fixture has no stray entries this test forgot to cover, and that a real
//! client invocation decodes to the pinned code end-to-end.

use soroban_sdk::{testutils::Address as _, Address, Env, Error};
use trade_executor::{ContractError, TradeExecutorContract, TradeExecutorContractClient};

const FIXTURE: &str = include_str!("../../../error-abi/public-error-abi.json");

/// Every `ContractError` variant. When adding a variant, add it here and
/// pin it with `python3 scripts/check_error_abi.py --update`.
const CONTRACT_ERRORS: &[(&str, ContractError)] = &[
    ("NotInitialized", ContractError::NotInitialized),
    ("PositionLimitReached", ContractError::PositionLimitReached),
    ("InsufficientBalance", ContractError::InsufficientBalance),
    ("InvalidAmount", ContractError::InvalidAmount),
    ("ReentrancyDetected", ContractError::ReentrancyDetected),
    ("Unauthorized", ContractError::Unauthorized),
    ("TradeNotFound", ContractError::TradeNotFound),
    ("SlippageExceeded", ContractError::SlippageExceeded),
    ("PositionPctTooHigh", ContractError::PositionPctTooHigh),
    ("OraclePriceStale", ContractError::OraclePriceStale),
    ("OracleUnavailable", ContractError::OracleUnavailable),
    (
        "DailyVolumeLimitExceeded",
        ContractError::DailyVolumeLimitExceeded,
    ),
    ("OracleNotWhitelisted", ContractError::OracleNotWhitelisted),
    (
        "CannotRemoveLastOracle",
        ContractError::CannotRemoveLastOracle,
    ),
    (
        "OpenInterestLimitReached",
        ContractError::OpenInterestLimitReached,
    ),
    ("DCAPlanNotFound", ContractError::DCAPlanNotFound),
    ("DCAPlanAlreadyExists", ContractError::DCAPlanAlreadyExists),
    ("SignalExpired", ContractError::SignalExpired),
    ("IntervalNotDue", ContractError::IntervalNotDue),
    ("NetworkCongestion", ContractError::NetworkCongestion),
    ("InsufficientLiquidity", ContractError::InsufficientLiquidity),
    ("CircuitBreakerActive", ContractError::CircuitBreakerActive),
    ("FeatureDisabled", ContractError::FeatureDisabled),
    ("ReplayDetected", ContractError::ReplayDetected),
    ("BelowMinimumTradeSize", ContractError::BelowMinimumTradeSize),
    ("GracePeriodExpired", ContractError::GracePeriodExpired),
    ("QueuedTradeNotFound", ContractError::QueuedTradeNotFound),
    ("NotTradeOwner", ContractError::NotTradeOwner),
    ("MaxRetriesExceeded", ContractError::MaxRetriesExceeded),
    (
        "ConfirmationDepthNotReached",
        ContractError::ConfirmationDepthNotReached,
    ),
    ("TooManyOpenPositions", ContractError::TooManyOpenPositions),
    ("NonceAlreadyUsed", ContractError::NonceAlreadyUsed),
    ("TradeExpired", ContractError::TradeExpired),
    ("ContractPaused", ContractError::ContractPaused),
    ("AssetNotRegistered", ContractError::AssetNotRegistered),
    ("IdenticalAssets", ContractError::IdenticalAssets),
    ("UnsupportedPair", ContractError::UnsupportedPair),
    (
        "AssetRegistryNotConfigured",
        ContractError::AssetRegistryNotConfigured,
    ),
    ("TokenOperationFailed", ContractError::TokenOperationFailed),
    (
        "InsufficientAllowance",
        ContractError::InsufficientAllowance,
    ),
];

fn fixture_contract_errors() -> serde_json::Map<String, serde_json::Value> {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is valid JSON");
    fixture["crates"]["trade_executor"]["ContractError"]
        .as_object()
        .expect("fixture pins trade_executor::ContractError")
        .clone()
}

#[test]
fn contract_error_codes_match_fixture() {
    let pinned = fixture_contract_errors();
    for (name, variant) in CONTRACT_ERRORS {
        let expected = pinned
            .get(*name)
            .unwrap_or_else(|| panic!("ContractError::{name} is not pinned in the fixture"))
            .as_u64()
            .unwrap();
        assert_eq!(
            *variant as u64, expected,
            "ContractError::{name} was renumbered — this is a breaking change to the public ABI"
        );
    }
}

/// Catches the inverse drift: a variant added to the fixture (e.g. via a
/// careless manual edit, or forgetting to extend `CONTRACT_ERRORS` above
/// after adding a new variant to the enum) that this test does not know
/// about, so a reordering/renumbering of it would otherwise go unnoticed.
#[test]
fn fixture_has_no_contract_error_missing_from_this_test() {
    let listed: std::collections::BTreeSet<&str> =
        CONTRACT_ERRORS.iter().map(|(n, _)| *n).collect();
    for name in fixture_contract_errors().keys() {
        assert!(
            listed.contains(name.as_str()),
            "fixture pins ContractError::{name} but this test does not cover it"
        );
    }
}

/// End-to-end: a real failing invocation through the generated client must
/// decode to the exact numeric code pinned in the fixture, not just the
/// symbolic variant the Rust enum happens to produce in this build.
#[test]
fn client_receives_the_pinned_not_initialized_code() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TradeExecutorContract, ());
    let client = TradeExecutorContractClient::new(&env, &contract_id);
    let governance = Address::generate(&env);

    // Calling an admin-gated entrypoint before `initialize()` must fail
    // with the pinned `NotInitialized` code (see
    // `trade_executor::oracle::require_admin`), proving the generated
    // client surfaces the committed ABI code end-to-end.
    let pinned = fixture_contract_errors()["NotInitialized"].as_u64().unwrap() as u32;
    match client.try_set_governance(&governance) {
        Err(Ok(err)) => {
            assert_eq!(err, ContractError::NotInitialized);
            assert_eq!(Error::from(err), Error::from_contract_error(pinned));
        }
        other => panic!("expected NotInitialized, got {other:?}"),
    }
}
