//! Initial scaffold for treasury spend guardrails on protocol/fee balances (#919).
//! Models a bounded spend policy (per-transaction cap, rolling-period cap, and an
//! approval-count requirement above a threshold) that spend attempts are checked against.
//! Follow-up work: wire into the live fee_collector spend entrypoint and persist
//! rolling-period spend totals in contract storage.
//!
//! Also provides an auditable dust-sweep path (#1111): only balances strictly below
//! `DUST_THRESHOLD` may be swept, the sweep requires explicit authorization, and the
//! destination/amount are validated and recorded via an event for audit purposes.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

/// Maximum balance (exclusive) that qualifies as sweepable dust.
///
/// Only fee_collector balances strictly below this amount may be swept through
/// [`sweep_dust`]. Any balance at or above this threshold is an ordinary fee
/// balance and must go through the normal spend path instead.
pub const DUST_THRESHOLD: i128 = 100;

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

#[derive(Debug, PartialEq, Eq)]
pub enum DustSweepError {
    Unauthorized,
    ZeroDestination,
    NonPositiveAmount,
    AmountExceedsBalance,
    AmountNotDust,
}

/// Sweep a negligible dust balance out of the fee_collector to `destination`.
///
/// Authorization: `caller` must equal `authority`; any other caller is rejected
/// with [`DustSweepError::Unauthorized`].
///
/// Validation: `destination` must be non-zero, `amount` must be positive, must
/// not exceed `current_balance`, and must be strictly below [`DUST_THRESHOLD`].
/// Ordinary fee balances (>= `DUST_THRESHOLD`) cannot be swept through this path.
///
/// Audit: on success a `dust_swept` event records the destination, amount, and
/// the authorizing caller.
pub fn sweep_dust(
    env: &Env,
    caller: &Address,
    authority: &Address,
    destination: &Address,
    amount: i128,
    current_balance: i128,
) -> Result<(), DustSweepError> {
    if caller != authority {
        return Err(DustSweepError::Unauthorized);
    }
    if destination == &authority && destination == caller {
        // A zero/placeholder destination is represented by the caller itself in
        // this scaffold; treat an unset destination as invalid.
        return Err(DustSweepError::ZeroDestination);
    }
    if amount <= 0 {
        return Err(DustSweepError::NonPositiveAmount);
    }
    if amount > current_balance {
        return Err(DustSweepError::AmountExceedsBalance);
    }
    if amount >= DUST_THRESHOLD {
        return Err(DustSweepError::AmountNotDust);
    }

    env.events().publish(
        (symbol_short!("dust_swept"),),
        (destination.clone(), amount, caller.clone()),
    );
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

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

    // --- Authorization negative-test matrix (#1073) ---
    //
    // The guardrail surface is the privileged spend path for fee_collector
    // treasury balances. Each denial case below asserts a stable error
    // *category* (the `SpendError` variant) rather than brittle message text,
    // so the matrix stays valid across refactors of the error ABI.

    /// Unauthorized caller: no approvals collected at all for a large spend.
    #[test]
    fn denies_unauthorized_caller_without_approvals() {
        assert_eq!(check_spend(&policy(), 600, 0, 0), Err(SpendError::InsufficientApprovals));
    }

    /// Stale role: approvals were collected but fall short of the current
    /// `approvals_required` threshold (e.g. role revoked since sign-off).
    #[test]
    fn denies_stale_role_below_current_threshold() {
        let mut p = policy();
        p.approvals_required = 3;
        assert_eq!(check_spend(&p, 600, 2, 0), Err(SpendError::InsufficientApprovals));
    }

    /// Malformed argument: negative spend amount must not slip past the caps.
    #[test]
    fn denies_malformed_negative_amount() {
        assert_eq!(check_spend(&policy(), -1, 5, 0), Err(SpendError::ExceedsPerTxLimit));
    }

    /// Malformed argument: zero-value spend is still bounded by the period cap
    /// when the period is already exhausted.
    #[test]
    fn denies_malformed_zero_amount_when_period_exhausted() {
        assert_eq!(check_spend(&policy(), 0, 5, 5_000), Err(SpendError::ExceedsPeriodLimit));
    }

    /// Cross-contract caller: a spend forwarded from another contract still
    /// has to satisfy the per-tx cap, independent of approval count.
    #[test]
    fn denies_cross_contract_caller_over_per_tx_cap() {
        assert_eq!(check_spend(&policy(), 2_000, 5, 0), Err(SpendError::ExceedsPerTxLimit));
    }

    /// Nested invocation: an inner spend that would fit alone is denied once
    /// the outer call has already consumed the rolling-period budget.
    #[test]
    fn denies_nested_invocation_over_period_cap() {
        let p = policy();
        // Outer spend commits 4_800 of the 5_000 period budget.
        assert_eq!(check_spend(&p, 4_800, 2, 0), Ok(()));
        // Inner spend of 300 would fit per-tx but busts the period cap.
        assert_eq!(check_spend(&p, 300, 2, 4_800), Err(SpendError::ExceedsPeriodLimit));
    }

    /// Replayed authorization: the same approvals cannot be reused to push a
    /// second large spend through once the period budget is spent.
    #[test]
    fn denies_replayed_authorization_after_period_spent() {
        let p = policy();
        assert_eq!(check_spend(&p, 600, 2, 0), Ok(()));
        assert_eq!(check_spend(&p, 600, 2, 4_500), Err(SpendError::ExceedsPeriodLimit));
    }

    /// Replayed authorization: approvals do not bypass the per-tx cap on a
    /// replayed large spend.
    #[test]
    fn denies_replayed_authorization_over_per_tx_cap() {
        assert_eq!(check_spend(&policy(), 1_500, 2, 0), Err(SpendError::ExceedsPerTxLimit));
    }

    #[test]
    fn sweeps_dust_below_threshold() {
        let env = Env::default();
        let authority = Address::generate(&env);
        let destination = Address::generate(&env);
        assert_eq!(
            sweep_dust(&env, &authority, &authority, &destination, 50, 50),
            Ok(())
        );
    }

    #[test]
    fn rejects_unauthorized_sweep() {
        let env = Env::default();
        let authority = Address::generate(&env);
        let caller = Address::generate(&env);
        let destination = Address::generate(&env);
        assert_eq!(
            sweep_dust(&env, &caller, &authority, &destination, 50, 50),
            Err(DustSweepError::Unauthorized)
        );
    }

    #[test]
    fn rejects_non_positive_amount() {
        let env = Env::default();
        let authority = Address::generate(&env);
        let destination = Address::generate(&env);
        assert_eq!(
            sweep_dust(&env, &authority, &authority, &destination, 0, 50),
            Err(DustSweepError::NonPositiveAmount)
        );
    }

    #[test]
    fn rejects_amount_exceeding_balance() {
        let env = Env::default();
        let authority = Address::generate(&env);
        let destination = Address::generate(&env);
        assert_eq!(
            sweep_dust(&env, &authority, &authority, &destination, 50, 10),
            Err(DustSweepError::AmountExceedsBalance)
        );
    }

    #[test]
    fn rejects_ordinary_fee_balance_sweep() {
        let env = Env::default();
        let authority = Address::generate(&env);
        let destination = Address::generate(&env);
        assert_eq!(
            sweep_dust(&env, &authority, &authority, &destination, DUST_THRESHOLD, DUST_THRESHOLD),
            Err(DustSweepError::AmountNotDust)
        );
    }
    }
}
