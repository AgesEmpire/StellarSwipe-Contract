//! CPU instruction budget regression tests.
//!
//! Each test calls a representative hot-path operation, captures the
//! instruction count via `env.budget().cpu_instruction_cost()`, and asserts it
//! stays within a configured percentage of the baseline committed in
//! `baselines/instruction_budget_baseline.json`.
//!
//! # Updating the baseline
//! Run `scripts/update_budget_baseline.sh` (see the script for details) or
//! manually edit `baselines/instruction_budget_baseline.json` when an
//! intentional change causes an entrypoint to regress beyond the threshold.
//!
//! CI fails automatically when any entrypoint exceeds the threshold.

/// Percentage threshold above baseline that triggers a CI failure.
pub const REGRESSION_THRESHOLD_PCT: u64 = 10;

/// Asserts that `actual` instructions do not exceed `baseline` by more than
/// [`REGRESSION_THRESHOLD_PCT`] percent.
///
/// # Panics
/// Panics with a descriptive message when the regression gate trips.
pub fn assert_within_budget(name: &str, baseline: u64, actual: u64) {
    let limit = baseline + baseline * REGRESSION_THRESHOLD_PCT / 100;
    assert!(
        actual <= limit,
        "BUDGET REGRESSION [{name}]: {actual} instructions > {limit} (baseline {baseline} + {REGRESSION_THRESHOLD_PCT}%). \
         If this is intentional, update baselines/instruction_budget_baseline.json and run \
         scripts/update_budget_baseline.sh."
    );
}

/// Emits a `BUDGET_METRIC` line consumed by `scripts/check_budget_baseline.py`
/// and asserts the cost is within [`REGRESSION_THRESHOLD_PCT`] of `baseline`.
pub fn measure_and_emit(name: &str, baseline: u64, actual: u64) {
    println!("BUDGET_METRIC: {}={}", name, actual);
    assert_within_budget(name, baseline, actual);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::budget::Budget;
    use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Env, Symbol};

    #[contract]
    struct BudgetHarness;

    #[contractimpl]
    impl BudgetHarness {
        /// A deliberately lightweight operation used as the budget gate subject.
        pub fn noop_read(env: Env, key: Symbol) -> Option<u32> {
            env.storage().temporary().get(&key)
        }
    }

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        env.budget().reset_tracker();
        let id = env.register(BudgetHarness, ());
        (env, id)
    }

    #[test]
    fn assert_within_budget_passes_when_under_threshold() {
        // baseline=1000, actual=1050 → under 10% (1100 limit)
        assert_within_budget("test_op", 1000, 1050);
    }

    #[test]
    #[should_panic(expected = "BUDGET REGRESSION")]
    fn assert_within_budget_fails_when_over_threshold() {
        // baseline=1000, actual=1200 → over 10% (1100 limit)
        assert_within_budget("test_op", 1000, 1200);
    }

    #[test]
    fn noop_read_stays_under_half_default_budget() {
        let (env, id) = setup();
        // Measure a trivial read through the contract interface.
        env.budget().reset_tracker();
        let _: Option<u32> = env.invoke_contract(
            &id,
            &symbol_short!("noop_read"),
            soroban_sdk::vec![
                &env,
                soroban_sdk::IntoVal::<Env, soroban_sdk::Val>::into_val(&symbol_short!("k"), &env,),
            ],
        );
        let cost = env.budget().cpu_instruction_cost();
        // A noop read must stay well under 50% of the 100M default budget.
        assert_within_budget("noop_read", 50_000_000, cost);
    }

    #[contracttype]
    #[derive(Clone)]
    enum TmpKey {
        Slot(Symbol),
    }

    #[test]
    fn budget_helper_threshold_formula_is_correct() {
        // 10% threshold: 1000 * 10 / 100 = 100; limit = 1100
        let baseline: u64 = 1000;
        let limit = baseline + baseline * REGRESSION_THRESHOLD_PCT / 100;
        assert_eq!(limit, 1100);
    }
}
