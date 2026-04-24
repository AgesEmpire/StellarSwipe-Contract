//! Commit-reveal helpers to **bind** a user’s trade parameters before execution.
//!
//! These functions do **not** by themselves stop ordering attacks inside a Stellar
//! validator’s mempool; they give integrators a canonical `SHA-256` over intent fields
//! so a future on-chain or off-chain “commit” phase can reference the same bytes.
//! See `docs/security/front_running_analysis.md`.

use soroban_sdk::{Address, Bytes, BytesN, Env, String};

/// `SHA-256( "sw_exec_v1" || user || signal_id || amount || min_out || salt
/// || valid_until_ledger )` as a [`BytesN<32>`].
///
/// - `min_out` — user-defined floor for received amount (slippage / MEV margin).
/// - `valid_until_ledger` — user expects execution by this ledger (inclusive);
///   contracts that adopt commit-reveal should reject reveals after this ledger.
/// - `salt` — high-entropy; clients should use a CSPRNG (or expand to 32 bytes in
///   a future version of this API).
pub fn hash_trade_intent(
    env: &Env,
    user: &Address,
    signal_id: u64,
    amount: i128,
    min_out: i128,
    salt: u64,
    valid_until_ledger: u32,
) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.append(&String::from_str(env, "sw_exec_v1").to_bytes());
    preimage.append(&user.to_string().to_bytes());
    preimage.append(&Bytes::from_array(
        env,
        &signal_id.to_be_bytes(),
    ));
    preimage.append(&Bytes::from_array(env, &amount.to_be_bytes()));
    preimage.append(&Bytes::from_array(env, &min_out.to_be_bytes()));
    preimage.append(&Bytes::from_array(env, &salt.to_be_bytes()));
    preimage.append(&Bytes::from_array(
        env,
        &valid_until_ledger.to_be_bytes(),
    ));
    env.crypto().sha256(&preimage).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn hash_is_deterministic() {
        let env = Env::default();
        let a = Address::generate(&env);
        let h1 = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        let h2 = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_changes_when_amount_changes() {
        let env = Env::default();
        let a = Address::generate(&env);
        let h1 = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        let h2 = hash_trade_intent(&env, &a, 5, 1_000_001, 900_000, 42, 1_000_000);
        assert_ne!(h1, h2);
    }
}
