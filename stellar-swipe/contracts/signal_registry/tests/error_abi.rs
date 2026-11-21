//! Public error ABI compatibility (issue #1222).
//!
//! `error-abi/public-error-abi.json` pins the numeric code of every public
//! `#[contracterror]` variant; `scripts/check_error_abi.py` enforces it
//! against the source in CI. These tests check the same fixture from the
//! compiled crate: the codes `AdminError` actually has, and the code a client
//! receives when a call fails.

use serde_json::Value as Json;
use signal_registry::{AdminError, SignalRegistry, SignalRegistryClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Error};

const FIXTURE: &str = include_str!("../../../error-abi/public-error-abi.json");

/// Every `AdminError` variant. When adding a variant, add it here and pin it
/// with `python3 scripts/check_error_abi.py --update`.
const ADMIN_ERRORS: &[(&str, AdminError)] = &[
    ("Unauthorized", AdminError::Unauthorized),
    ("AlreadyInitialized", AdminError::AlreadyInitialized),
    ("NotInitialized", AdminError::NotInitialized),
    ("InvalidParameter", AdminError::InvalidParameter),
    ("TradingPaused", AdminError::TradingPaused),
    ("PauseExpired", AdminError::PauseExpired),
    ("InvalidFeeRate", AdminError::InvalidFeeRate),
    ("InvalidRiskParameter", AdminError::InvalidRiskParameter),
    ("InsufficientSignatures", AdminError::InsufficientSignatures),
    ("DuplicateSigner", AdminError::DuplicateSigner),
    ("InvalidAssetPair", AdminError::InvalidAssetPair),
    ("CannotFollowSelf", AdminError::CannotFollowSelf),
    ("RateLimitExceeded", AdminError::RateLimitExceeded),
    ("SignalLimitExceeded", AdminError::SignalLimitExceeded),
    ("InvalidTimestamp", AdminError::InvalidTimestamp),
    ("ScheduleTooFarFuture", AdminError::ScheduleTooFarFuture),
    ("ScheduleLimitReached", AdminError::ScheduleLimitReached),
    ("ScheduleNotFound", AdminError::ScheduleNotFound),
    ("NotScheduleOwner", AdminError::NotScheduleOwner),
    (
        "CircuitBreakerTriggered",
        AdminError::CircuitBreakerTriggered,
    ),
    ("StakeBelowMinimum", AdminError::StakeBelowMinimum),
    ("PendingAdminNotFound", AdminError::PendingAdminNotFound),
    ("PendingAdminExpired", AdminError::PendingAdminExpired),
    ("ReentrancyDetected", AdminError::ReentrancyDetected),
    (
        "RequiresMultisigApproval",
        AdminError::RequiresMultisigApproval,
    ),
    ("ProposalNotFound", AdminError::ProposalNotFound),
    ("AlreadyApproved", AdminError::AlreadyApproved),
    ("ProposalNotApproved", AdminError::ProposalNotApproved),
    ("TimelockNotElapsed", AdminError::TimelockNotElapsed),
    (
        "ProposalAlreadyExecuted",
        AdminError::ProposalAlreadyExecuted,
    ),
    ("ProposalCancelled", AdminError::ProposalCancelled),
    ("TooManyProposals", AdminError::TooManyProposals),
    ("CooldownNotElapsed", AdminError::CooldownNotElapsed),
    (
        "IncompatibleContractVersion",
        AdminError::IncompatibleContractVersion,
    ),
    (
        "IncompatibleStorageLayout",
        AdminError::IncompatibleStorageLayout,
    ),
];

fn fixture_admin_errors() -> serde_json::Map<String, Json> {
    let fixture: Json = serde_json::from_str(FIXTURE).expect("fixture is valid JSON");
    fixture["crates"]["signal_registry"]["AdminError"]
        .as_object()
        .expect("fixture pins signal_registry::AdminError")
        .clone()
}

#[test]
fn admin_error_codes_match_fixture() {
    let pinned = fixture_admin_errors();
    for (name, variant) in ADMIN_ERRORS {
        let expected = pinned
            .get(*name)
            .unwrap_or_else(|| panic!("AdminError::{name} is not pinned in the fixture"))
            .as_u64()
            .unwrap();
        assert_eq!(
            *variant as u64, expected,
            "AdminError::{name} was renumbered"
        );
    }
}

#[test]
fn fixture_has_no_admin_error_missing_from_the_crate() {
    let listed: std::collections::BTreeSet<&str> = ADMIN_ERRORS.iter().map(|(n, _)| *n).collect();
    for name in fixture_admin_errors().keys() {
        assert!(
            listed.contains(name.as_str()),
            "fixture pins AdminError::{name} but this test does not cover it"
        );
    }
}

#[test]
fn clients_receive_the_pinned_code() {
    let env = Env::default();
    env.mock_all_auths();
    let client = SignalRegistryClient::new(&env, &env.register(SignalRegistry, ()));
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let pinned = fixture_admin_errors()["AlreadyInitialized"]
        .as_u64()
        .unwrap() as u32;
    match client.try_initialize(&admin) {
        Err(Ok(err)) => {
            assert_eq!(err, AdminError::AlreadyInitialized);
            assert_eq!(Error::from(err), Error::from_contract_error(pinned));
        }
        other => panic!("expected AlreadyInitialized, got {other:?}"),
    }
}
