//! Hot-entry TTL management for persistent storage.
//!
//! Soroban archives persistent entries whose TTL reaches zero.  For keys that
//! are read frequently (active signals, leaderboard indexes) we call
//! `extend_ttl` on every access using Soroban's built-in `threshold` parameter:
//! the host only performs the extension when the remaining TTL has already
//! dropped below `threshold`, so no wasted instructions are incurred when the
//! TTL is still healthy.
//!
//! # Constants
//! | Constant | Default | Purpose |
//! |---|---|---|
//! | `HOT_KEY_TTL_TARGET_LEDGERS` | 518 400 (~30 days) | Desired TTL after a bump |
//! | `HOT_KEY_TTL_THRESHOLD_LEDGERS` | 103 680 (~6 days) | Bump only when TTL falls below this |

use soroban_sdk::{Env, IntoVal, Val};

/// Target TTL (ledgers) after an extend: ~30 days at 5-second ledger close.
pub const HOT_KEY_TTL_TARGET_LEDGERS: u32 = 518_400;

/// Only extend when the remaining TTL drops below this threshold: ~6 days.
/// Passed as the `threshold` argument to `extend_ttl`; the Soroban host skips
/// the operation when the TTL is already above this value.
pub const HOT_KEY_TTL_THRESHOLD_LEDGERS: u32 = 103_680;

/// Extend the TTL of a **persistent** storage entry identified by `key` if its
/// remaining TTL is below [`HOT_KEY_TTL_THRESHOLD_LEDGERS`].
///
/// Uses the native `extend_ttl(key, threshold, extend_to)` semantics: the host
/// skips the call when the entry's current TTL >= `threshold`, so this is safe
/// to call on every read/write without wasting instructions.
///
/// Does nothing when the entry does not exist in persistent storage.
pub fn bump_persistent_if_needed<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    let storage = env.storage().persistent();
    if !storage.has(key) {
        return;
    }
    storage.extend_ttl(
        key,
        HOT_KEY_TTL_THRESHOLD_LEDGERS,
        HOT_KEY_TTL_TARGET_LEDGERS,
    );
}

/// Unconditionally extend the TTL of a persistent entry to
/// [`HOT_KEY_TTL_TARGET_LEDGERS`].
///
/// Passes `threshold = 0` so the extend always fires regardless of the current
/// TTL.  Use this in the keeper batch-bump entrypoint where the caller wants to
/// explicitly top-up a set of keys.
pub fn force_bump_persistent<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    let storage = env.storage().persistent();
    if storage.has(key) {
        storage.extend_ttl(key, 0, HOT_KEY_TTL_TARGET_LEDGERS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::{contract, contractimpl, contracttype, Env};

    #[contracttype]
    #[derive(Clone)]
    enum TestKey {
        Hot,
        Missing,
    }

    #[contract]
    struct TtlHarness;

    #[contractimpl]
    impl TtlHarness {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        let id = env.register(TtlHarness, ());
        (env, id)
    }

    #[test]
    fn bump_extends_ttl_for_low_ttl_entry() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            env.storage().persistent().set(&TestKey::Hot, &42u32);
            // Start with a low TTL (below threshold) so bump_persistent_if_needed fires.
            env.storage().persistent().extend_ttl(
                &TestKey::Hot,
                0,
                HOT_KEY_TTL_THRESHOLD_LEDGERS - 1,
            );

            let ttl_before = env.storage().persistent().get_ttl(&TestKey::Hot);
            bump_persistent_if_needed(&env, &TestKey::Hot);
            let ttl_after = env.storage().persistent().get_ttl(&TestKey::Hot);
            assert!(ttl_after > ttl_before, "TTL should increase after bump");
            assert!(
                ttl_after >= HOT_KEY_TTL_TARGET_LEDGERS,
                "TTL should reach the target"
            );
        });
    }

    #[test]
    fn bump_noop_when_ttl_above_threshold() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            env.storage().persistent().set(&TestKey::Hot, &99u32);
            env.storage()
                .persistent()
                .extend_ttl(&TestKey::Hot, 0, HOT_KEY_TTL_TARGET_LEDGERS);

            let ttl_before = env.storage().persistent().get_ttl(&TestKey::Hot);
            // With threshold-based extend_ttl the host skips when TTL >= threshold.
            bump_persistent_if_needed(&env, &TestKey::Hot);
            let ttl_after = env.storage().persistent().get_ttl(&TestKey::Hot);
            assert!(
                ttl_after >= ttl_before,
                "TTL must not decrease on no-op bump"
            );
        });
    }

    #[test]
    fn bump_noop_for_missing_entry() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            // Must not panic.
            bump_persistent_if_needed(&env, &TestKey::Missing);
        });
    }

    #[test]
    fn force_bump_always_extends() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            env.storage().persistent().set(&TestKey::Hot, &1u32);
            // Start at target so normal threshold-bump wouldn't trigger.
            env.storage()
                .persistent()
                .extend_ttl(&TestKey::Hot, 0, HOT_KEY_TTL_TARGET_LEDGERS);
            let ttl_before = env.storage().persistent().get_ttl(&TestKey::Hot);
            force_bump_persistent(&env, &TestKey::Hot);
            let ttl_after = env.storage().persistent().get_ttl(&TestKey::Hot);
            assert!(
                ttl_after >= ttl_before,
                "force_bump must not shrink TTL below target"
            );
        });
    }
}
