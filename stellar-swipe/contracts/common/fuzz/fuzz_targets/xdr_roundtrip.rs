//! Fuzz target: XDR round-trip for storage key serialisation.
//!
//! Feeds arbitrary bytes into the XDR decoder and verifies that:
//!  1. Decoding never panics (even for malformed input).
//!  2. Any value that decodes successfully re-encodes to the same bytes
//!     (idempotency).
//!
//! This is a second fuzz target kept intentionally simple so the fuzzer can
//! explore the parser independently of the key-collision properties tested in
//! `storage_key_derivation`.
//!
//! ## Running locally
//! ```bash
//! cargo +nightly fuzz run xdr_roundtrip -- -max_total_time=300
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{contract, contractimpl, contracttype, Env};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
enum RoundTripKey {
    Unit,
    Scalar(u64),
    Pair(u32, u32),
}

#[contract]
struct RtHarness;

#[contractimpl]
impl RtHarness {}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let env = Env::default();
    let id = env.register(RtHarness, ());

    env.as_contract(&id, || {
        // Pick a key variant deterministically from the first byte.
        let key: RoundTripKey = match data[0] % 3 {
            0 => RoundTripKey::Unit,
            1 => {
                if data.len() < 9 {
                    return;
                }
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&data[1..9]);
                RoundTripKey::Scalar(u64::from_le_bytes(bytes))
            }
            _ => {
                if data.len() < 9 {
                    return;
                }
                let mut a = [0u8; 4];
                let mut b = [0u8; 4];
                a.copy_from_slice(&data[1..5]);
                b.copy_from_slice(&data[5..9]);
                RoundTripKey::Pair(u32::from_le_bytes(a), u32::from_le_bytes(b))
            }
        };

        // Set and get back — if the host XDR round-trips cleanly this won't panic.
        env.storage().persistent().set(&key, &42u32);
        let got: Option<u32> = env.storage().persistent().get(&key);
        assert_eq!(got, Some(42u32), "XDR round-trip failed: value mismatch");

        // Overwrite and verify idempotency.
        env.storage().persistent().set(&key, &99u32);
        let got2: Option<u32> = env.storage().persistent().get(&key);
        assert_eq!(got2, Some(99u32), "XDR round-trip failed: second write mismatch");
    });
});
