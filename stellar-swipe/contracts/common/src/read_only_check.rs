//! Shared test helper for asserting a query entrypoint is actually
//! read-only (Issue #1336).
//!
//! Query/view-style entrypoints are expected not to write persistent
//! storage, extend TTLs, or publish events merely because a caller read
//! data — doing so increases the caller's resource cost (and, for events,
//! leaks information) for what looks like a free read. This module gives
//! every contract's test suite one assertion to call around a candidate
//! read-only invocation instead of each crate hand-rolling its own
//! before/after event-count comparison (see, e.g., the pre-existing
//! `governance::test_simulation::simulation_is_read_only_no_storage_writes`
//! and `user_portfolio::test_portfolio_export::export_is_read_only`, which
//! each implemented an ad hoc version of this check).
//!
//! # What this checks
//! The Soroban host does not expose a generic "list every persistent key
//! touched" API to test code, so this helper uses the two externally
//! observable proxies that *are* exposed: the event stream
//! (`Env::events().all()`) and repeated-call result stability. A query
//! that performs a hidden storage write with no corresponding behavioral
//! change (no new event, same return value on every call) would not be
//! caught by event-count alone — call [`assert_read_only_stable`] (which
//! also re-invokes the query and compares results) for that stronger
//! guarantee whenever the query's return type implements `PartialEq`.
//!
//! # Example
//! ```ignore
//! use stellar_swipe_common::read_only_check::assert_no_events_published;
//!
//! let result = assert_no_events_published(&env, || client.get_signal(&signal_id));
//! ```

use soroban_sdk::Env;

/// Invokes `f` and asserts that it published no events, i.e. that `f` is
/// consistent with being a pure/read-only call from the perspective of the
/// contract's public event surface.
///
/// Returns `f`'s result so callers can chain further assertions on it.
///
/// # Panics
/// Panics with a descriptive message naming `label` if any event was
/// published during the call.
pub fn assert_no_events_published<T>(env: &Env, label: &str, f: impl FnOnce() -> T) -> T {
    let before = env.events().all().len();
    let result = f();
    let after = env.events().all().len();
    assert_eq!(
        after, before,
        "expected '{label}' to be read-only, but it published {} event(s); \
         a query must not publish events merely because a caller read data \
         (Issue #1336). If this entrypoint legitimately needs to write or \
         extend TTL, document and test that explicitly instead of treating \
         it as read-only.",
        after.saturating_sub(before)
    );
    result
}

/// Stronger version of [`assert_no_events_published`]: in addition to the
/// no-events check, invokes `f` a second time and asserts the two results
/// are equal, catching a hidden storage mutation that changes behavior on
/// a second call (e.g. an auto-incrementing counter bumped as a read-time
/// side effect) even though it happened not to emit an event.
///
/// # Panics
/// Panics if either invocation publishes an event, or if the two results
/// differ.
pub fn assert_read_only_stable<T: PartialEq + core::fmt::Debug + Clone>(
    env: &Env,
    label: &str,
    f: impl Fn() -> T,
) -> T {
    let first = assert_no_events_published(env, label, || f());
    let second = assert_no_events_published(env, label, || f());
    assert_eq!(
        first, second,
        "expected '{label}' to return the same result on repeated calls \
         with no intervening state change (Issue #1336); a differing result \
         suggests the query itself mutated state it then read back"
    );
    first
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, symbol_short, Env};

    #[contract]
    struct PureQueryStub;

    #[contractimpl]
    impl PureQueryStub {
        pub fn read_value(_env: Env) -> u32 {
            42
        }

        pub fn sneaky_write(env: Env) -> u32 {
            env.events()
                .publish((symbol_short!("sneaky"),), 1u32);
            42
        }
    }

    #[test]
    fn passes_for_a_genuinely_read_only_call() {
        let env = Env::default();
        let id = env.register(PureQueryStub, ());
        let client = PureQueryStubClient::new(&env, &id);
        let value = assert_read_only_stable(&env, "read_value", || client.read_value());
        assert_eq!(value, 42);
    }

    #[test]
    #[should_panic(expected = "expected 'sneaky_write' to be read-only")]
    fn fails_when_the_call_publishes_an_event() {
        let env = Env::default();
        let id = env.register(PureQueryStub, ());
        let client = PureQueryStubClient::new(&env, &id);
        assert_no_events_published(&env, "sneaky_write", || client.sneaky_write());
    }
}
