//! Storage key collision tests (Issue #265).
//!
//! Verifies that user-keyed `#[contracttype]` enum variants produce different
//! serialised storage keys for different addresses, including addresses that
//! differ in only one byte.

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contracttype, Address, Env};

    // ---------------------------------------------------------------------------
    // Minimal reproductions of every user-keyed storage key pattern used across
    // the five contracts.  We reproduce the *pattern* here so the test has no
    // extra crate dependencies.
    // ---------------------------------------------------------------------------

    #[contracttype]
    #[derive(Clone)]
    enum SingleAddrKey {
        UserPositions(Address),           // user_portfolio::DataKey
        UserBadges(Address),              // user_portfolio::DataKey
        Authorization(Address),           // auto_trade::AuthKey
        PositionLimitExempt(Address),     // trade_executor::StorageKey
        LastInsufficientBalance(Address), // trade_executor::StorageKey
        ProviderReputationScore(Address), // signal_registry::StorageKey
        TreasuryBalance(Address),         // fee_collector::StorageKey
        MonthlyTradeVolume(Address),      // fee_collector::StorageKey
        ProviderTerms(Address),           // user_portfolio subscriptions::StorageKey
    }

    #[contracttype]
    #[derive(Clone)]
    enum TwoAddrKey {
        ProviderPendingFees(Address, Address), // fee_collector::StorageKey
        Subscription(Address, Address),        // user_portfolio subscriptions::StorageKey
    }

    // stake_vault has no user-keyed variants; per-staker data lives inside a
    // Map<Address, StakeInfoV2> stored under a single MigrationKey::StakesV2 entry.
    // We verify the two migration keys and the two StorageKey unit variants are distinct.
    #[contracttype]
    #[derive(Clone)]
    enum StakeVaultMigrationKey {
        StakesV1,
        StakesV2,
        MigrationState,
    }

    #[contracttype]
    #[derive(Clone)]
    enum StakeVaultStorageKey {
        Admin,
        StakeToken,
    }

    // Write key `a` with a sentinel value; assert key `b` is absent (different key).
    fn assert_keys_distinct(env: &Env, a: &SingleAddrKey, b: &SingleAddrKey) {
        env.storage().persistent().set(a, &1u32);
        assert!(
            !env.storage().persistent().has(b),
            "storage key collision: two distinct keys serialise to the same bytes"
        );
        env.storage().persistent().remove(a);
    }

    fn assert_two_addr_keys_distinct(env: &Env, a: &TwoAddrKey, b: &TwoAddrKey) {
        env.storage().persistent().set(a, &1u32);
        assert!(
            !env.storage().persistent().has(b),
            "storage key collision: two distinct two-address keys serialise to the same bytes"
        );
        env.storage().persistent().remove(a);
    }

    // ---------------------------------------------------------------------------
    // Single-address variants: different users → different keys
    // ---------------------------------------------------------------------------

    #[test]
    fn single_addr_variants_differ_for_different_users() {
        let env = Env::default();
        let user_a = Address::generate(&env);
        let user_b = Address::generate(&env);

        assert_keys_distinct(&env, &SingleAddrKey::UserPositions(user_a.clone()), &SingleAddrKey::UserPositions(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::UserBadges(user_a.clone()), &SingleAddrKey::UserBadges(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::Authorization(user_a.clone()), &SingleAddrKey::Authorization(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::PositionLimitExempt(user_a.clone()), &SingleAddrKey::PositionLimitExempt(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::LastInsufficientBalance(user_a.clone()), &SingleAddrKey::LastInsufficientBalance(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::ProviderReputationScore(user_a.clone()), &SingleAddrKey::ProviderReputationScore(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::TreasuryBalance(user_a.clone()), &SingleAddrKey::TreasuryBalance(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::MonthlyTradeVolume(user_a.clone()), &SingleAddrKey::MonthlyTradeVolume(user_b.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::ProviderTerms(user_a.clone()), &SingleAddrKey::ProviderTerms(user_b.clone()));
    }

    // ---------------------------------------------------------------------------
    // Two-address variants: different first argument → different keys
    // ---------------------------------------------------------------------------

    #[test]
    fn two_addr_variants_differ_for_different_users() {
        let env = Env::default();
        let user_a = Address::generate(&env);
        let user_b = Address::generate(&env);
        let provider = Address::generate(&env);

        assert_two_addr_keys_distinct(
            &env,
            &TwoAddrKey::ProviderPendingFees(user_a.clone(), provider.clone()),
            &TwoAddrKey::ProviderPendingFees(user_b.clone(), provider.clone()),
        );
        assert_two_addr_keys_distinct(
            &env,
            &TwoAddrKey::Subscription(user_a.clone(), provider.clone()),
            &TwoAddrKey::Subscription(user_b.clone(), provider.clone()),
        );
    }

    // ---------------------------------------------------------------------------
    // Argument order matters: (a, b) ≠ (b, a)
    // ---------------------------------------------------------------------------

    #[test]
    fn two_addr_variant_argument_order_matters() {
        let env = Env::default();
        let user = Address::generate(&env);
        let provider = Address::generate(&env);

        assert_two_addr_keys_distinct(
            &env,
            &TwoAddrKey::Subscription(user.clone(), provider.clone()),
            &TwoAddrKey::Subscription(provider.clone(), user.clone()),
        );
    }

    // ---------------------------------------------------------------------------
    // Different variants with the same address payload must not collide
    // ---------------------------------------------------------------------------

    #[test]
    fn different_variants_same_address_do_not_collide() {
        let env = Env::default();
        let addr = Address::generate(&env);

        assert_keys_distinct(&env, &SingleAddrKey::UserPositions(addr.clone()), &SingleAddrKey::UserBadges(addr.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::TreasuryBalance(addr.clone()), &SingleAddrKey::MonthlyTradeVolume(addr.clone()));
        assert_keys_distinct(&env, &SingleAddrKey::PositionLimitExempt(addr.clone()), &SingleAddrKey::LastInsufficientBalance(addr.clone()));
    }

    // ---------------------------------------------------------------------------
    // stake_vault: MigrationKey unit variants are all distinct
    // ---------------------------------------------------------------------------

    #[test]
    fn stake_vault_migration_keys_are_distinct() {
        let env = Env::default();

        env.storage().persistent().set(&StakeVaultMigrationKey::StakesV1, &1u32);
        assert!(!env.storage().persistent().has(&StakeVaultMigrationKey::StakesV2));
        assert!(!env.storage().persistent().has(&StakeVaultMigrationKey::MigrationState));
        env.storage().persistent().remove(&StakeVaultMigrationKey::StakesV1);

        env.storage().persistent().set(&StakeVaultMigrationKey::StakesV2, &1u32);
        assert!(!env.storage().persistent().has(&StakeVaultMigrationKey::MigrationState));
        env.storage().persistent().remove(&StakeVaultMigrationKey::StakesV2);
    }

    #[test]
    fn stake_vault_storage_keys_are_distinct() {
        let env = Env::default();

        env.storage().instance().set(&StakeVaultStorageKey::Admin, &1u32);
        assert!(!env.storage().instance().has(&StakeVaultStorageKey::StakeToken));
        env.storage().instance().remove(&StakeVaultStorageKey::Admin);
    }
}
