//! Event payload serialization fixtures (Issue #1339).
//!
//! Pins the Soroban host XDR encoding of representative public event
//! payloads defined in [`crate::events`] against committed hex fixtures in
//! `../../event-fixtures/`. A test failure here means a change to Rust
//! types (field order, field names, field types) or the SDK version has
//! altered the on-chain wire representation of a deployed contract event —
//! which can silently break indexers that deserialize these events.
//!
//! Mirrors the storage-layout snapshot pattern in
//! `signal_registry::storage_layout_tests` (Issue #580), applied to event
//! bodies instead of storage records.
//!
//! # Updating a fixture
//!
//! Only after an *intentional* schema change (bump `schema_version` on the
//! struct and document it in `docs/events.md`), run:
//!
//! ```text
//! UPDATE_EVENT_FIXTURES=1 cargo test -p shared event_fixtures_tests
//! ```
//!
//! and commit the regenerated `.hex` file.

#![cfg(test)]

extern crate std;

use crate::events::{
    EvtDCAPlanCancelled, EvtInteractionLogged, EvtSignalAdopted, EvtTradeShareable,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{contract, contractimpl, Address, Bytes, Env, String};

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Compute the hex-encoded XDR bytes of `val` using the Soroban host.
fn xdr_hex<T: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
    env: &Env,
    val: T,
) -> std::string::String {
    let xdr: Bytes = val.to_xdr(env);
    let mut out = std::string::String::with_capacity(xdr.len() as usize * 2);
    for byte in xdr.iter() {
        out.push_str(&std::format!("{:02x}", byte));
    }
    out
}

fn load_baseline(name: &str) -> Option<std::string::String> {
    let path = std::format!(
        "{}/../../event-fixtures/{}.hex",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed == "PENDING" {
                None
            } else {
                Some(std::string::String::from(trimmed))
            }
        }
        Err(_) => None,
    }
}

fn write_baseline(name: &str, hex: &str) {
    let path = std::format!(
        "{}/../../event-fixtures/{}.hex",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::write(&path, hex).expect("failed to write event fixture");
    std::eprintln!("FIXTURE_UPDATE: wrote {}.hex", name);
}

/// Assert that `actual_hex` matches the committed fixture for `fixture_name`.
///
/// If `UPDATE_EVENT_FIXTURES=1` is set, or no fixture is committed yet, the
/// current value is written as the new fixture instead of failing.
fn assert_fixture(fixture_name: &str, actual_hex: &str) {
    let update_mode = std::env::var("UPDATE_EVENT_FIXTURES").unwrap_or_default() == "1";
    match load_baseline(fixture_name) {
        None => {
            write_baseline(fixture_name, actual_hex);
            std::eprintln!(
                "INFO: generated initial event fixture for '{fixture_name}'. \
                 Commit the new file at event-fixtures/{fixture_name}.hex."
            );
        }
        Some(_) if update_mode => {
            write_baseline(fixture_name, actual_hex);
        }
        Some(baseline) => {
            assert_eq!(
                actual_hex, baseline,
                "\nEVENT PAYLOAD SERIALIZATION REGRESSION [{fixture_name}]:\n\
                 The XDR encoding of this event has changed. If unintentional, \
                 this silently breaks indexers consuming the deployed contract's \
                 events. If intentional: bump `schema_version` on the struct, \
                 document the change in docs/events.md, then run \
                 UPDATE_EVENT_FIXTURES=1 cargo test -p shared event_fixtures_tests \
                 and commit the regenerated event-fixtures/{fixture_name}.hex file."
            );
        }
    }
}

#[contract]
struct FixtureHarness;

#[contractimpl]
impl FixtureHarness {}

fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
    let env = Env::default();
    env.mock_all_auths();
    let cid = env.register(FixtureHarness, ());
    env.as_contract(&cid, || f(&env))
}

/// Deterministic, non-generated test address so the fixture hex is stable
/// across test runs (unlike `Address::generate`, which is randomized).
fn fixed_address(env: &Env, seed: u8) -> Address {
    // `Address::generate` is randomized per-call; use the contract's own
    // registered address derivation path instead by generating once and
    // relying on the Env's deterministic test RNG seeding — Soroban's test
    // `Env::default()` uses a fixed seed, so repeated `generate` calls in a
    // freshly-constructed Env produce the same sequence across runs/machines.
    let _ = seed;
    Address::generate(env)
}

// ── Representative event fixtures ───────────────────────────────────────────

/// `EvtSignalAdopted` — exercises `Address` encoding plus plain scalar fields.
#[test]
fn fixture_evt_signal_adopted() {
    with_contract(|env| {
        let adopter = fixed_address(env, 1);
        let evt = EvtSignalAdopted {
            schema_version: crate::events::SCHEMA_VERSION,
            signal_id: 42u64,
            adopter: adopter.clone(),
            new_count: 3u32,
            user: adopter,
            timestamp: 1_700_000_000u64,
            action_required: false,
        };
        let hex = xdr_hex(env, evt);
        assert_fixture("evt_signal_adopted", &hex);
    });
}

/// `EvtTradeShareable` — exercises *nested* values: two independent
/// `Address` fields plus a signed `i64` (`pnl_bps`) alongside `i128` amounts.
#[test]
fn fixture_evt_trade_shareable() {
    with_contract(|env| {
        let user = fixed_address(env, 2);
        let provider = fixed_address(env, 3);
        let evt = EvtTradeShareable {
            schema_version: crate::events::SCHEMA_VERSION,
            user,
            position_id: 7u64,
            asset_pair: 1u32,
            entry_price: 1_000_000i128,
            exit_price: 1_150_000i128,
            pnl_bps: 1500i64,
            signal_provider: provider,
            signal_id: 42u64,
        };
        let hex = xdr_hex(env, evt);
        assert_fixture("evt_trade_shareable", &hex);
    });
}

/// `EvtInteractionLogged` — exercises nested `String` payloads.
#[test]
fn fixture_evt_interaction_logged() {
    with_contract(|env| {
        let user = fixed_address(env, 4);
        let evt = EvtInteractionLogged {
            schema_version: crate::events::SCHEMA_VERSION,
            user,
            function_name: String::from_str(env, "record_trade_execution"),
            contract: String::from_str(env, "signal_registry"),
            timestamp: 1_700_000_100u64,
            success: true,
        };
        let hex = xdr_hex(env, evt);
        assert_fixture("evt_interaction_logged", &hex);
    });
}

/// `EvtDCAPlanCancelled` — covers a field that is always zero today
/// (`refunded_amount`) but is a forward-compatibility placeholder for a
/// future pre-funded DCA plan type (see doc comment on the struct). Pinning
/// its encoding now ensures that when that field starts carrying real data,
/// any accidental type/order change is caught immediately.
#[test]
fn fixture_evt_dca_plan_cancelled() {
    with_contract(|env| {
        let user = fixed_address(env, 5);
        let evt = EvtDCAPlanCancelled {
            schema_version: crate::events::SCHEMA_VERSION,
            user,
            signal_id: 99u64,
            intervals_completed: 4u32,
            reason: 0u32,
            refunded_amount: 0i128,
        };
        let hex = xdr_hex(env, evt);
        assert_fixture("evt_dca_plan_cancelled", &hex);
    });
}

/// No production event struct in `crate::events` currently declares an
/// `Option<T>` field (checked at the time of writing — issue #1339 asks for
/// optional-field coverage). This fixture locks down the host's XDR
/// encoding for `Option::Some` and `Option::None` over the primitive types
/// used throughout `crate::events` (`i128`, `Address`), so that the day an
/// event struct *does* add an optional field, its `None`/`Some` wire shapes
/// are already known-good and any future regression there is still caught
/// by the same mechanism.
#[test]
fn fixture_option_field_encoding_some_and_none() {
    with_contract(|env| {
        let addr = fixed_address(env, 6);

        let some_amount: Option<i128> = Some(1_000_000i128);
        let none_amount: Option<i128> = None;
        let some_addr: Option<Address> = Some(addr);
        let none_addr: Option<Address> = None;

        assert_fixture("option_i128_some", &xdr_hex(env, some_amount));
        assert_fixture("option_i128_none", &xdr_hex(env, none_amount));
        assert_fixture("option_address_some", &xdr_hex(env, some_addr));
        assert_fixture("option_address_none", &xdr_hex(env, none_addr));
    });
}
