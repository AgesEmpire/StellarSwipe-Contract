//! Instruction budget awareness and call-depth guards for complex workflows.
//!
//! Soroban imposes hard instruction-count and call-depth limits per transaction.
//! This module provides:
//!
//! - `MAX_CALL_DEPTH`   – maximum nesting depth for cross-contract calls.
//! - `check_call_depth` – panics when the guard would be exceeded.
//! - `BudgetGuard`      – checks that a workflow step does not exceed a
//!   configurable instruction-budget estimate before proceeding.
//!
//! # Usage
//!
//! ```ignore
//! // At the entry point of an expensive workflow:
//! budget_guard::check_call_depth(&env, current_depth)?;
//! budget_guard::check_budget(&env, WORKFLOW_BUDGET_ESTIMATE)?;
//! ```

#![allow(dead_code)]

use soroban_sdk::{contracttype, Env};
use stellar_swipe_common::storage_crud::{crud_get_or, crud_set, StorageTier};

// ── Constants ────────────────────────────────────────────────────────────────

/// Soroban enforces a maximum call stack depth of 10.  We cap internal logic
/// at a lower value so that overhead from the runtime itself and any outer
/// entry-point frames still fits within the platform limit.
pub const MAX_CALL_DEPTH: u32 = 6;

/// Conservative per-operation instruction estimate used as the default
/// budget ceiling when no override is stored.  Adjust via
/// `set_budget_ceiling` to tune for specific deployments.
pub const DEFAULT_BUDGET_CEILING: u64 = 2_500_000;

// ── Storage ──────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum BudgetGuardKey {
    /// Contract-wide instruction budget ceiling (instance storage).
    BudgetCeiling,
}

/// Return the configured instruction-budget ceiling, or the default.
pub fn get_budget_ceiling(env: &Env) -> u64 {
    crud_get_or(
        env,
        StorageTier::Instance,
        &BudgetGuardKey::BudgetCeiling,
        DEFAULT_BUDGET_CEILING,
    )
}

/// Persist a new budget ceiling (admin-only gate should be applied by caller).
pub fn set_budget_ceiling(env: &Env, ceiling: u64) {
    crud_set(
        env,
        StorageTier::Instance,
        &BudgetGuardKey::BudgetCeiling,
        &ceiling,
    );
}

// ── Guard functions ───────────────────────────────────────────────────────────

/// Assert that `current_depth` is below [`MAX_CALL_DEPTH`].
///
/// # Errors
/// Panics with `"call depth limit exceeded"` when the check fails.
pub fn check_call_depth(env: &Env, current_depth: u32) {
    let _ = env; // env available for future event emission
    if current_depth >= MAX_CALL_DEPTH {
        panic!("call depth limit exceeded");
    }
}

/// Assert that `estimated_instructions` does not exceed the stored ceiling.
///
/// # Errors
/// Panics with `"instruction budget exceeded"` when the check fails.
pub fn check_budget(env: &Env, estimated_instructions: u64) {
    let ceiling = get_budget_ceiling(env);
    if estimated_instructions > ceiling {
        panic!("instruction budget exceeded");
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    fn fresh_env() -> Env {
        Env::default()
    }

    #[test]
    fn call_depth_within_limit_passes() {
        let env = fresh_env();
        check_call_depth(&env, 0);
        check_call_depth(&env, MAX_CALL_DEPTH - 1);
    }

    #[test]
    #[should_panic(expected = "call depth limit exceeded")]
    fn call_depth_at_limit_panics() {
        let env = fresh_env();
        check_call_depth(&env, MAX_CALL_DEPTH);
    }

    #[test]
    #[should_panic(expected = "call depth limit exceeded")]
    fn call_depth_above_limit_panics() {
        let env = fresh_env();
        check_call_depth(&env, MAX_CALL_DEPTH + 5);
    }

    #[test]
    fn budget_within_default_ceiling_passes() {
        let env = fresh_env();
        let _cid = env.register_contract(None, crate::AutoTradeContract);
        check_budget(&env, 0);
        check_budget(&env, DEFAULT_BUDGET_CEILING);
    }

    #[test]
    #[should_panic(expected = "instruction budget exceeded")]
    fn budget_above_default_ceiling_panics() {
        let env = fresh_env();
        let _cid = env.register_contract(None, crate::AutoTradeContract);
        check_budget(&env, DEFAULT_BUDGET_CEILING + 1);
    }

    #[test]
    fn custom_ceiling_is_respected() {
        let env = fresh_env();
        let _cid = env.register_contract(None, crate::AutoTradeContract);
        set_budget_ceiling(&env, 100_000);
        check_budget(&env, 100_000); // exactly at ceiling → ok
    }

    #[test]
    #[should_panic(expected = "instruction budget exceeded")]
    fn custom_ceiling_exceeded_panics() {
        let env = fresh_env();
        let _cid = env.register_contract(None, crate::AutoTradeContract);
        set_budget_ceiling(&env, 100_000);
        check_budget(&env, 100_001);
    }

    #[test]
    fn get_set_budget_ceiling_roundtrip() {
        let env = fresh_env();
        let _cid = env.register_contract(None, crate::AutoTradeContract);
        assert_eq!(get_budget_ceiling(&env), DEFAULT_BUDGET_CEILING);
        set_budget_ceiling(&env, 999_999);
        assert_eq!(get_budget_ceiling(&env), 999_999);
    }

    #[test]
    fn call_depth_uses_env_parameter() {
        // Verify the function signature accepts an env reference without panic.
        let env = fresh_env();
        let _ = Address::generate(&env); // ensure env is used
        check_call_depth(&env, 0); // depth 0 always passes
    }
}
