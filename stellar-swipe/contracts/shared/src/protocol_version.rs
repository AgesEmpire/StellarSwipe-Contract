//! Issue #1334: minimum supported Soroban protocol version per contract.
//!
//! Soroban's host protocol version (`env.ledger().protocol_version()`)
//! changes the semantics available to a contract (new host functions, cost
//! model changes, etc.). A contract built against host functionality that
//! only exists from protocol `N` onward is not safe to invoke on a network
//! still running protocol `< N`.
//!
//! This module gives every contract a read-only, no-state-change query that
//! reports the protocol range it was built and tested against, so deployers
//! can check compatibility with `simulate`/`get_network` *before* ever
//! submitting a state-changing invocation. The minimum is a single source of
//! truth here — each contract's query just re-exports it — so bumping the
//! floor can never drift between contracts that are meant to share one.

use soroban_sdk::Env;

/// Lowest Soroban protocol version this codebase is built and tested
/// against. Bump this (and the corresponding CI matrix entry) only after
/// verifying every contract against the new minimum.
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u32 = 21;

/// The protocol version this codebase currently targets/is tested against
/// in CI as the upper, "current", end of the supported range.
pub const CURRENT_TARGET_PROTOCOL_VERSION: u32 = 23;

/// `(min_supported, current_target)` — the inclusive range of Soroban
/// protocol versions this contract is expected to behave correctly on.
pub fn supported_protocol_range() -> (u32, u32) {
    (
        MIN_SUPPORTED_PROTOCOL_VERSION,
        CURRENT_TARGET_PROTOCOL_VERSION,
    )
}

/// Whether the network this `env` is running on satisfies the minimum
/// supported protocol version. Pure read — no state access, no auth, safe to
/// call via `simulateTransaction` before submitting anything.
pub fn is_protocol_supported(env: &Env) -> bool {
    env.ledger().protocol_version() >= MIN_SUPPORTED_PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, Address, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        let id = env.register(TestContract, ());
        (env, id)
    }

    #[test]
    fn range_is_well_ordered() {
        let (min, current) = supported_protocol_range();
        assert!(min <= current);
        assert_eq!(min, MIN_SUPPORTED_PROTOCOL_VERSION);
    }

    #[test]
    fn minimum_protocol_version_is_supported() {
        let (env, id) = setup();
        env.ledger().set_protocol_version(MIN_SUPPORTED_PROTOCOL_VERSION);
        env.as_contract(&id, || {
            assert!(is_protocol_supported(&env));
        });
    }

    #[test]
    fn current_target_protocol_version_is_supported() {
        let (env, id) = setup();
        env.ledger()
            .set_protocol_version(CURRENT_TARGET_PROTOCOL_VERSION);
        env.as_contract(&id, || {
            assert!(is_protocol_supported(&env));
        });
    }

    #[test]
    fn below_minimum_protocol_version_is_rejected() {
        let (env, id) = setup();
        env.ledger()
            .set_protocol_version(MIN_SUPPORTED_PROTOCOL_VERSION - 1);
        env.as_contract(&id, || {
            assert!(!is_protocol_supported(&env));
        });
    }
}
