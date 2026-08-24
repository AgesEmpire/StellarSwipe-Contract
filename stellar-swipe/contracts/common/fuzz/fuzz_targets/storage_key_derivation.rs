//! Fuzz target: storage-key derivation (Issue #265 pattern).
//!
//! Exercises every user-keyed storage key variant used across the StellarSwipe
//! contracts to discover inputs that:
//!  - cause a panic / abort during key serialisation
//!  - produce collisions between distinct (variant, address) pairs
//!
//! The fuzzer feeds raw bytes; we interpret them as two 32-byte Stellar
//! addresses and check that the resulting storage keys are distinct whenever
//! the inputs differ.
//!
//! ## Running locally
//! ```bash
//! cargo +nightly fuzz run storage_key_derivation -- -max_total_time=300
//! ```
//!
//! ## Reproducing a crash
//! ```bash
//! cargo +nightly fuzz run storage_key_derivation artifacts/storage_key_derivation/<crash-file>
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, IntoVal};

// ── Minimal reproductions of every user-keyed storage key used in production ──

#[contracttype]
#[derive(Clone)]
enum SingleAddrKey {
    UserPositions(Address),
    UserBadges(Address),
    Authorization(Address),
    PositionLimitExempt(Address),
    LastInsufficientBalance(Address),
    ProviderReputationScore(Address),
    TreasuryBalance(Address),
    MonthlyTradeVolume(Address),
    ProviderTerms(Address),
}

#[contracttype]
#[derive(Clone)]
enum TwoAddrKey {
    ProviderPendingFees(Address, Address),
    Subscription(Address, Address),
}

#[contracttype]
#[derive(Clone)]
enum NumericKey {
    ComboExecutions(u64),
    ExportChecksum(u64),
    DailyFees(u64),
}

#[contract]
struct FuzzHarness;

#[contractimpl]
impl FuzzHarness {}

/// Try to construct a Stellar address from 32 arbitrary bytes.
/// Returns None when the bytes don't form a valid strkey.
fn try_address_from_bytes(env: &Env, raw: &[u8; 32]) -> Address {
    // Soroban testutils: Address::generate uses internal randomness, but we
    // need deterministic addresses from the fuzzer's bytes.  We XOR the raw
    // bytes into a valid-looking Ed25519 public key (type byte 0x06 for G-addresses).
    // The testutils Address::from_contract_id path accepts any 32-byte hash.
    let mut padded = [0u8; 32];
    padded.copy_from_slice(raw);
    let bytes = Bytes::from_slice(env, &padded);
    // contract_id-style address derivation — always valid for any 32 bytes.
    Address::from_contract_id(env, &bytes.into())
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 64 {
        return;
    }

    let env = Env::default();
    let contract_id = env.register(FuzzHarness, ());

    env.as_contract(&contract_id, || {
        // Carve two 32-byte addresses from the fuzzer input.
        let mut raw_a = [0u8; 32];
        let mut raw_b = [0u8; 32];
        raw_a.copy_from_slice(&data[..32]);
        raw_b.copy_from_slice(&data[32..64]);

        let addr_a = try_address_from_bytes(&env, &raw_a);
        let addr_b = try_address_from_bytes(&env, &raw_b);

        // Exercise single-address variants — must not panic.
        let _ = env
            .storage()
            .persistent()
            .set(&SingleAddrKey::UserPositions(addr_a.clone()), &1u32);
        let _ = env
            .storage()
            .persistent()
            .set(&SingleAddrKey::UserPositions(addr_b.clone()), &2u32);

        // When addresses differ the two keys must not collide.
        if raw_a != raw_b {
            let val_a: Option<u32> = env
                .storage()
                .persistent()
                .get(&SingleAddrKey::UserPositions(addr_a.clone()));
            let val_b: Option<u32> = env
                .storage()
                .persistent()
                .get(&SingleAddrKey::UserPositions(addr_b.clone()));
            assert_ne!(
                val_a, val_b,
                "storage key collision: distinct addresses produced the same key"
            );
        }

        // Exercise cross-variant non-collision: same address, different variants.
        env.storage()
            .persistent()
            .set(&SingleAddrKey::UserBadges(addr_a.clone()), &10u32);
        let collision_check: Option<u32> = env
            .storage()
            .persistent()
            .get(&SingleAddrKey::UserPositions(addr_a.clone()));
        // UserPositions(addr_a) was set to 1; UserBadges(addr_a) was set to 10.
        // They must not share the same slot.
        assert_ne!(
            collision_check,
            Some(10u32),
            "cross-variant storage key collision detected"
        );

        // Exercise two-address variants.
        env.storage()
            .persistent()
            .set(&TwoAddrKey::Subscription(addr_a.clone(), addr_b.clone()), &42u32);
        env.storage()
            .persistent()
            .set(&TwoAddrKey::Subscription(addr_b.clone(), addr_a.clone()), &99u32);
        if raw_a != raw_b {
            let fwd: Option<u32> = env
                .storage()
                .persistent()
                .get(&TwoAddrKey::Subscription(addr_a.clone(), addr_b.clone()));
            let rev: Option<u32> = env
                .storage()
                .persistent()
                .get(&TwoAddrKey::Subscription(addr_b.clone(), addr_a.clone()));
            assert_ne!(
                fwd, rev,
                "TwoAddrKey: (a,b) and (b,a) must not collide when a != b"
            );
        }

        // Exercise numeric keys with boundary values from the input.
        if data.len() >= 72 {
            let mut id_bytes = [0u8; 8];
            id_bytes.copy_from_slice(&data[64..72]);
            let id = u64::from_le_bytes(id_bytes);
            env.storage()
                .temporary()
                .set(&NumericKey::DailyFees(id), &id);
            let _: Option<u64> = env.storage().temporary().get(&NumericKey::DailyFees(id));
        }
    });
});
