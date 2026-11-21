//! Reproducible release fixture (Issue #1341).
//!
//! Unlike every other test in this crate, which exercises contracts through
//! their *native* `#[contract]` implementation registered directly in the
//! test `Env`, this fixture deploys the **exact optimized WASM artifact**
//! that `stellar-swipe/scripts/build.sh` produces at
//! `target/wasm-optimized/signal_registry.wasm` and invokes a handful of
//! representative entrypoints against it. This catches behavior
//! differences between native test execution and the artifact that is
//! actually shipped to a network (e.g. WASM-only numeric/host-function
//! edge cases, or an optimization pass that changes observable behavior).
//!
//! # Running locally
//!
//! The optimized WASM must exist first:
//!
//! ```text
//! cd stellar-swipe && ./scripts/build.sh
//! cargo test -p integration_tests --test test_release_fixture
//! ```
//!
//! If the artifact is not present (e.g. a normal `cargo test --workspace`
//! run that never built WASM), this test prints a skip notice and passes
//! trivially rather than failing the whole suite — CI's dedicated release
//! job (see `.github/workflows/ci.yml`, step "Run release fixture against
//! optimized WASM") always builds the artifact first, so there it always
//! runs for real.

extern crate std;

use soroban_sdk::{
    testutils::Address as _, Address, Env, String, Vec,
};

/// Path to the optimized WASM artifact `scripts/build.sh` produces, relative
/// to this crate's manifest directory.
fn wasm_path() -> std::string::String {
    std::format!(
        "{}/../../target/wasm-optimized/signal_registry.wasm",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn release_fixture_signal_registry_optimized_wasm() {
    let path = wasm_path();
    let wasm_bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            std::eprintln!(
                "SKIP: release fixture — optimized WASM not found at {path}. \
                 Run `cd stellar-swipe && ./scripts/build.sh` first, or rely on \
                 the dedicated CI job which always builds it before this test."
            );
            return;
        }
    };

    // Record the artifact hash alongside the behavioral result, per the
    // issue's "What done looks like" — CI's release-fixture step greps this
    // line out of the test output and records it into
    // release-fixture-results/latest.json alongside the pass/fail outcome.
    // Shells out to `sha256sum` (present on every CI/dev Linux box this
    // workspace already requires) rather than pulling in a new hashing
    // dependency just for this diagnostic line.
    if let Ok(output) = std::process::Command::new("sha256sum").arg(&path).output() {
        if output.status.success() {
            let line = std::string::String::from_utf8_lossy(&output.stdout);
            let hash = line.split_whitespace().next().unwrap_or("unknown");
            std::eprintln!("RELEASE_FIXTURE_ARTIFACT_HASH: signal_registry.wasm sha256={hash}");
        }
    }

    let env = Env::default();
    env.mock_all_auths();

    // Deploy the real optimized artifact instead of the native #[contract].
    #[allow(deprecated)]
    let contract_id = env.register_contract_wasm(None, wasm_bytes.as_slice());

    let client = signal_registry::SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    // ── Representative entrypoint: initialize ──────────────────────────
    client.initialize(&admin);

    // ── Representative entrypoint: stake + create_signal ───────────────
    client.stake_tokens(&provider, &200_000_000i128);

    let expiry = 3_600u64;
    let events_before = env.events().all().len();
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &signal_registry::SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, "release fixture rationale"),
        &expiry,
        &signal_registry::SignalCategory::SWING,
        &Vec::new(&env),
        &signal_registry::RiskLevel::Medium,
    );
    let events_after = env.events().all().len();

    // Native expectation (matches `test_complete_user_journey`): the first
    // signal created gets id 1, and creating a signal emits at least one
    // event. A WASM-specific regression (e.g. an id off-by-one from a
    // numeric host-function discrepancy, or a dropped event) would trip
    // these.
    assert_eq!(
        signal_id, 1u64,
        "optimized WASM must assign the same first signal id as native code"
    );
    assert!(
        events_after > events_before,
        "optimized WASM must emit at least one event for create_signal, matching native behavior"
    );

    // ── Representative entrypoint: get_signal round-trip ───────────────
    let signal = client
        .get_signal(&signal_id)
        .expect("signal must be readable back through the WASM artifact");
    assert_eq!(signal.provider, provider);
    assert_eq!(signal.adoption_count, 0u32);
    assert_eq!(signal.status, signal_registry::SignalStatus::Active);

    // ── Representative error-path entrypoint: duplicate init rejected ──
    let reinit = client.try_initialize(&admin);
    assert!(
        reinit.is_err(),
        "optimized WASM must reject double-initialization just like native code"
    );
}
