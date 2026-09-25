//! Initial scaffold for treasury spend guardrails on protocol/fee balances (#919).
//! Models a bounded spend policy (per-transaction cap, rolling-period cap, and an
//! approval-count requirement above a threshold) that spend attempts are checked against.
//! Follow-up work: wire into the live fee_collector spend entrypoint and persist
//! rolling-period spend totals in contract storage.

use soroban_sdk::contracttype;

#[derive(Clone)]
#[contracttype]
pub struct SpendPolicy {
    pub max_per_tx: i128,
    pub max_per_period: i128,
    pub period_seconds: u64,
    /// Spends at or above this amount require `approvals_required` sign-offs.
    pub large_spend_threshold: i128,
    pub approvals_required: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SpendError {
    ExceedsPerTxLimit,
    ExceedsPeriodLimit,
    InsufficientApprovals,
    /// The withdrawal would reduce protected treasury reserves below the
    /// configured solvency floor. State is left unchanged.
    BelowSolvencyFloor,
}

/// Pure guardrail check: given the policy, the amount being spent, approvals
/// collected so far, and spend already committed within the current period,
/// returns Ok(()) only if the spend is within every bound.
pub fn check_spend(
    policy: &SpendPolicy,
    amount: i128,
    approvals_collected: u32,
    period_spent_so_far: i128,
) -> Result<(), SpendError> {
    if amount > policy.max_per_tx {
        return Err(SpendError::ExceedsPerTxLimit);
    }
    if period_spent_so_far + amount > policy.max_per_period {
        return Err(SpendError::ExceedsPeriodLimit);
    }
    if amount >= policy.large_spend_threshold && approvals_collected < policy.approvals_required {
        return Err(SpendError::InsufficientApprovals);
    }
    Ok(())
}

/// Enforces the treasury solvency invariant: a withdrawal of `amount` from
/// `treasury_balance` must not reduce protected reserves below `solvency_floor`.
///
/// This is a pure precondition check so it can be evaluated atomically with the
/// withdrawal: callers must run it *before* mutating any balance, and only apply
/// the debit when it returns `Ok(())`. On violation it returns the stable
/// [`SpendError::BelowSolvencyFloor`] error and no state is touched.
pub fn check_solvency(
    treasury_balance: i128,
    amount: i128,
    solvency_floor: i128,
) -> Result<(), SpendError> {
    if treasury_balance - amount < solvency_floor {
        return Err(SpendError::BelowSolvencyFloor);
    }
    Ok(())
}

/// Combined precondition for a withdrawal: validates the spend policy bounds and
/// the treasury solvency invariant together. Returns `Ok(())` only when both
/// checks pass, so the caller can apply the debit atomically without partial
/// state mutation.
pub fn check_withdrawal(
    policy: &SpendPolicy,
    treasury_balance: i128,
    amount: i128,
    approvals_collected: u32,
    period_spent_so_far: i128,
    solvency_floor: i128,
) -> Result<(), SpendError> {
    check_spend(policy, amount, approvals_collected, period_spent_so_far)?;
    check_solvency(treasury_balance, amount, solvency_floor)
}

#[cfg(test)]
mod test {
    use super::*;

    fn policy() -> SpendPolicy {
        SpendPolicy {
            max_per_tx: 1_000,
            max_per_period: 5_000,
            period_seconds: 86_400,
            large_spend_threshold: 500,
            approvals_required: 2,
        }
    }

    #[test]
    fn allows_small_spend_within_bounds() {
        assert_eq!(check_spend(&policy(), 100, 0, 0), Ok(()));
    }

    #[test]
    fn blocks_spend_over_per_tx_cap() {
        assert_eq!(check_spend(&policy(), 1_001, 5, 0), Err(SpendError::ExceedsPerTxLimit));
    }

    #[test]
    fn blocks_spend_over_period_cap() {
        assert_eq!(check_spend(&policy(), 900, 5, 4_500), Err(SpendError::ExceedsPeriodLimit));
    }

    #[test]
    fn blocks_large_spend_without_enough_approvals() {
        assert_eq!(check_spend(&policy(), 600, 1, 0), Err(SpendError::InsufficientApprovals));
    }

    #[test]
    fn allows_large_spend_with_enough_approvals() {
        assert_eq!(check_spend(&policy(), 600, 2, 0), Ok(()));
    }

    #[test]
    fn allows_withdrawal_landing_exactly_on_floor() {
        // balance 1_000, withdraw 400, floor 600 -> exactly at floor, allowed.
        assert_eq!(check_solvency(1_000, 400, 600), Ok(()));
    }

    #[test]
    fn rejects_withdrawal_below_floor() {
        // balance 1_000, withdraw 401, floor 600 -> 599 < 600, rejected.
        assert_eq!(check_solvency(1_000, 401, 600), Err(SpendError::BelowSolvencyFloor));
    }

    #[test]
    fn allows_withdrawal_after_reserve_replenishment() {
        // Initially below floor, then reserves are topped up and the same
        // withdrawal becomes valid.
        assert_eq!(check_solvency(500, 400, 600), Err(SpendError::BelowSolvencyFloor));
        assert_eq!(check_solvency(1_500, 400, 600), Ok(()));
    }

    #[test]
    fn combined_withdrawal_enforces_both_policy_and_solvency() {
        // Policy-valid but solvency-violating withdrawal is rejected.
        assert_eq!(
            check_withdrawal(&policy(), 1_000, 400, 2, 0, 600),
            Err(SpendError::BelowSolvencyFloor)
        );
        // Policy-valid and solvency-valid withdrawal is allowed.
        assert_eq!(check_withdrawal(&policy(), 1_000, 400, 2, 0, 500), Ok(()));
    }
}
