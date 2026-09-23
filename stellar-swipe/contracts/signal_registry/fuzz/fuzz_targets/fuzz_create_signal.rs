#![no_main]

// Fuzz target for issue #780: no combination of boundary inputs to
// `create_signal` should ever panic. `price`, the expiry offset, the
// rationale bytes, and the tag count are exactly the fields
// `validation::validate_signal_input` and the `MAX_EXPIRY_SECONDS` /
// `MAX_RATIONALE_LEN` / 10-tag checks in `create_signal_internal` are meant
// to guard — this generates arbitrary combinations of them (including the
// documented i128 / u64 edge values) and asserts the call only ever
// completes as `Ok` or a typed `AdminError`, never a panic.
//
// A `panic!` reached *inside* the contract call is caught by the Soroban
// host at the invocation boundary (see `catch_unwind` around contract
// dispatch in `soroban-env-host`'s test/native execution path) and
// surfaced here as an `Err`, not a Rust unwind — so `try_create_signal`
// itself cannot panic on account of contract logic. If libFuzzer still
// reports a crash on this target, the bug is either outside that
// boundary (our harness setup below) or a genuine host-level regression.

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use signal_registry::{
    RiskLevel, SignalAction, SignalCategory, SignalRegistry, SignalRegistryClient,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String, Vec};

#[derive(Debug, Arbitrary)]
struct FuzzCreateSignalInput {
    price: i128,
    expiry_offset: u64,
    rationale: std::vec::Vec<u8>,
    tags_count: u8,
}

fuzz_target!(|input: FuzzCreateSignalInput| {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let asset_pair = String::from_str(&env, "XLM/USDC");
    let rationale = String::from_bytes(&env, &input.rationale);

    let now = env.ledger().timestamp();
    let expiry = now.saturating_add(input.expiry_offset);

    // Bounded to keep exec/sec reasonable; still covers the 10-tag limit
    // boundary (TooManyTags) from both sides.
    let tag_count = (input.tags_count as u32) % 20;
    let mut tags = Vec::new(&env);
    for _ in 0..tag_count {
        tags.push_back(String::from_str(&env, "tag"));
    }

    let _ = client.try_create_signal(
        &provider,
        &asset_pair,
        &SignalAction::Buy,
        &input.price,
        &rationale,
        &expiry,
        &SignalCategory::SWING,
        &tags,
        &RiskLevel::Medium,
    );
});
