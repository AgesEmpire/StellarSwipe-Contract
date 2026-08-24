//! Minimum-liquidity threshold guard for pooled-fund withdrawals (Issue #591).
//!
//! Shared by any contract that holds a pool of funds on behalf of multiple
//! participants (staking pools, trading liquidity pools, etc.). Integrating
//! contracts call [`validate_withdrawal`] before transferring funds out on the
//! normal withdrawal path — before any state changes — and
//! [`emit_emergency_withdrawal`] from an admin-gated bypass path that is allowed
//! to drain the pool below the threshold when genuinely necessary.
//!
//! This module only holds the threshold config and guard logic; the integrating
//! contract owns the actual token balance and transfer, since pool identity and
//! admin auth are contract-specific concerns.

#![allow(dead_code)]

use soroban_sdk::{contracterror, contracttype, Address, Env, Symbol};

/// Default minimum-liquidity threshold for pools without an explicit override:
/// no restriction. Pools opt in to a floor via [`set_min_liquidity_threshold`].
pub const DEFAULT_MIN_LIQUIDITY_THRESHOLD: i128 = 0;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum LiquidityPoolError {
    /// The withdrawal would bring the pool's balance below its configured minimum.
    BelowMinimumLiquidity = 1,
}

#[contracttype]
#[derive(Clone)]
enum LiquidityPoolKey {
    MinThreshold(Address),
}

/// Effective minimum-liquidity threshold for `pool_id` (override, or
/// [`DEFAULT_MIN_LIQUIDITY_THRESHOLD`]).
pub fn get_min_liquidity_threshold(env: &Env, pool_id: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&LiquidityPoolKey::MinThreshold(pool_id.clone()))
        .unwrap_or(DEFAULT_MIN_LIQUIDITY_THRESHOLD)
}

/// Set the minimum-liquidity threshold for `pool_id`. The integrating contract
/// is responsible for admin authorization before calling this.
pub fn set_min_liquidity_threshold(env: &Env, pool_id: &Address, threshold: i128) {
    env.storage()
        .instance()
        .set(&LiquidityPoolKey::MinThreshold(pool_id.clone()), &threshold);
}

/// Reject a withdrawal that would bring `current_balance` below the configured
/// threshold for `pool_id`. Call before any state changes on the normal
/// withdrawal path; does not apply to the emergency bypass path.
pub fn validate_withdrawal(
    env: &Env,
    pool_id: &Address,
    current_balance: i128,
    withdraw_amount: i128,
) -> Result<(), LiquidityPoolError> {
    let threshold = get_min_liquidity_threshold(env, pool_id);
    let remaining = current_balance.saturating_sub(withdraw_amount);
    if remaining < threshold {
        Err(LiquidityPoolError::BelowMinimumLiquidity)
    } else {
        Ok(())
    }
}

/// Emit a distinct, audit-friendly event marking an admin emergency withdrawal
/// that bypassed the minimum-liquidity guard. Call from the integrating
/// contract's admin-only emergency withdrawal entrypoint, after the admin's
/// `require_auth()` has already succeeded.
pub fn emit_emergency_withdrawal(
    env: &Env,
    pool_id: &Address,
    admin: &Address,
    amount: i128,
    remaining_balance: i128,
) {
    env.events().publish(
        (
            Symbol::new(env, "liquidity_pool"),
            Symbol::new(env, "emergency_withdrawal"),
        ),
        (pool_id.clone(), admin.clone(), amount, remaining_balance),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl,
        testutils::Address as _,
        token::{self, StellarAssetClient},
        Env,
    };

    // ── Pure guard-logic tests ──────────────────────────────────────────────
    //
    // These exercise storage-backed helpers directly, so each runs inside a
    // registered contract's storage context via `env.as_contract`.

    fn storage_ctx() -> (Env, Address) {
        let env = Env::default();
        let id = env.register(PoolHarness, ());
        (env, id)
    }

    #[test]
    fn withdrawal_below_threshold_is_rejected() {
        let (env, id) = storage_ctx();
        env.as_contract(&id, || {
            let pool_id = Address::generate(&env);
            set_min_liquidity_threshold(&env, &pool_id, 1_000);

            let err = validate_withdrawal(&env, &pool_id, 1_500, 600);
            assert_eq!(err, Err(LiquidityPoolError::BelowMinimumLiquidity));
        });
    }

    #[test]
    fn withdrawal_exactly_at_threshold_is_allowed() {
        let (env, id) = storage_ctx();
        env.as_contract(&id, || {
            let pool_id = Address::generate(&env);
            set_min_liquidity_threshold(&env, &pool_id, 1_000);

            assert_eq!(validate_withdrawal(&env, &pool_id, 1_500, 500), Ok(()));
        });
    }

    #[test]
    fn withdrawal_above_threshold_is_allowed() {
        let (env, id) = storage_ctx();
        env.as_contract(&id, || {
            let pool_id = Address::generate(&env);
            set_min_liquidity_threshold(&env, &pool_id, 1_000);

            assert_eq!(validate_withdrawal(&env, &pool_id, 5_000, 100), Ok(()));
        });
    }

    #[test]
    fn default_threshold_is_zero_when_unset() {
        let (env, id) = storage_ctx();
        env.as_contract(&id, || {
            let pool_id = Address::generate(&env);
            assert_eq!(get_min_liquidity_threshold(&env, &pool_id), 0);
            // Draining a pool to exactly zero is fine under the default (no floor).
            assert_eq!(validate_withdrawal(&env, &pool_id, 100, 100), Ok(()));
        });
    }

    #[test]
    fn admin_can_update_threshold() {
        let (env, id) = storage_ctx();
        env.as_contract(&id, || {
            let pool_id = Address::generate(&env);
            set_min_liquidity_threshold(&env, &pool_id, 1_000);
            assert_eq!(get_min_liquidity_threshold(&env, &pool_id), 1_000);

            set_min_liquidity_threshold(&env, &pool_id, 2_000);
            assert_eq!(get_min_liquidity_threshold(&env, &pool_id), 2_000);
        });
    }

    // ── End-to-end harness: a minimal pool contract adopting the guard ──────

    #[contract]
    struct PoolHarness;

    #[contracttype]
    #[derive(Clone)]
    enum HarnessKey {
        Admin,
        Token,
    }

    #[contractimpl]
    impl PoolHarness {
        pub fn initialize(env: Env, admin: Address, token: Address) {
            env.storage().instance().set(&HarnessKey::Admin, &admin);
            env.storage().instance().set(&HarnessKey::Token, &token);
        }

        pub fn set_threshold(env: Env, pool_id: Address, threshold: i128) {
            let admin: Address = env.storage().instance().get(&HarnessKey::Admin).unwrap();
            admin.require_auth();
            set_min_liquidity_threshold(&env, &pool_id, threshold);
        }

        /// Normal withdrawal path: guarded by the minimum-liquidity threshold.
        pub fn withdraw(
            env: Env,
            caller: Address,
            pool_id: Address,
            amount: i128,
        ) -> Result<(), LiquidityPoolError> {
            caller.require_auth();
            let token: Address = env.storage().instance().get(&HarnessKey::Token).unwrap();
            let balance = token::Client::new(&env, &token).balance(&env.current_contract_address());
            validate_withdrawal(&env, &pool_id, balance, amount)?;
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &caller,
                &amount,
            );
            Ok(())
        }

        /// Admin-only emergency withdrawal: bypasses the threshold guard entirely,
        /// logged distinctly via `emit_emergency_withdrawal`.
        pub fn emergency_withdraw(env: Env, pool_id: Address, amount: i128) {
            let admin: Address = env.storage().instance().get(&HarnessKey::Admin).unwrap();
            admin.require_auth();
            let token: Address = env.storage().instance().get(&HarnessKey::Token).unwrap();
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &admin,
                &amount,
            );
            let remaining =
                token::Client::new(&env, &token).balance(&env.current_contract_address());
            emit_emergency_withdrawal(&env, &pool_id, &admin, amount, remaining);
        }
    }

    fn setup_harness(pool_balance: i128) -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let issuer = Address::generate(&env);
        let token = env.register_stellar_asset_contract_v2(issuer).address();
        let pool_id = env.register(PoolHarness, ());

        StellarAssetClient::new(&env, &token).mint(&pool_id, &pool_balance);

        let client = PoolHarnessClient::new(&env, &pool_id);
        client.initialize(&admin, &token);

        (env, pool_id, token)
    }

    #[test]
    fn harness_withdrawal_blocked_by_guard() {
        let (env, pool_id, _token) = setup_harness(1_000);
        let client = PoolHarnessClient::new(&env, &pool_id);
        client.set_threshold(&pool_id, &800);

        let caller = Address::generate(&env);
        let result = client.try_withdraw(&caller, &pool_id, &300);
        assert_eq!(result, Err(Ok(LiquidityPoolError::BelowMinimumLiquidity)));
    }

    #[test]
    fn harness_withdrawal_allowed_above_threshold() {
        let (env, pool_id, token) = setup_harness(1_000);
        let client = PoolHarnessClient::new(&env, &pool_id);
        client.set_threshold(&pool_id, &800);

        let caller = Address::generate(&env);
        client.withdraw(&caller, &pool_id, &200);

        assert_eq!(token::Client::new(&env, &token).balance(&pool_id), 800);
    }

    #[test]
    fn harness_emergency_withdraw_bypasses_guard_and_emits_event() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;

        let (env, pool_id, token) = setup_harness(1_000);
        let client = PoolHarnessClient::new(&env, &pool_id);
        client.set_threshold(&pool_id, &800);

        // A normal withdrawal of 900 would breach the 800 threshold...
        let caller = Address::generate(&env);
        assert!(client.try_withdraw(&caller, &pool_id, &900).is_err());

        // ...but the admin emergency path bypasses it entirely.
        client.emergency_withdraw(&pool_id, &900);

        let events = env.events().all();
        let e = events.last().unwrap();
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "liquidity_pool"));
        assert_eq!(t1, Symbol::new(&env, "emergency_withdrawal"));
        assert_eq!(token::Client::new(&env, &token).balance(&pool_id), 100);
    }
}
