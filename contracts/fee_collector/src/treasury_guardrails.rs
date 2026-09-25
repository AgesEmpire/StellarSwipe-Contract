//! Initial scaffold for treasury spend guardrails on protocol/fee balances (#919).
//! Models a bounded spend policy (per-transaction cap, rolling-period cap, and an
//! approval-count requirement above a threshold) that spend attempts are checked against.
//! Follow-up work: wire into the live fee_collector spend entrypoint and persist
//! rolling-period spend totals in contract storage.
//!
//! Token transfer hardening (#1087): token operations must validate the value
//! returned by the token contract and treat any non-success response as a failure
//! that leaves accounting unchanged. See [`check_token_transfer_result`] and
//! [`apply_token_transfer`] for the supported token behaviors.

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

/// Error returned when a token operation does not report success.
#[derive(Debug, PartialEq, Eq)]
pub enum TokenError {
    /// The token returned `false` (or a non-`true` value) instead of success.
    TransferFailed,
    /// The token returned a value that is not a well-formed boolean.
    MalformedResponse,
}

/// Validates the value returned by a token `transfer`/`transfer_from`/`approve`
/// call. Supported token behaviors:
///
/// * Well-behaved tokens return `true` on success and `false` on failure.
/// * Tokens that signal failure without trapping return `false`; this is treated
///   as [`TokenError::TransferFailed`].
/// * Tokens that return a malformed (non-boolean) value are rejected as
///   [`TokenError::MalformedResponse`] rather than being optimistically accepted.
/// * Tokens that trap (panic) abort the whole call; no accounting is committed
///   because the host rolls back the invocation.
///
/// Only an explicit `true` is accepted as success.
pub fn check_token_transfer_result(result: Result<bool, ()>) -> Result<(), TokenError> {
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(TokenError::TransferFailed),
        Err(()) => Err(TokenError::MalformedResponse),
    }
}

/// Applies a token transfer to a running balance, committing the new balance
/// only when the token reports success. On any failure the original balance is
/// returned unchanged, so failure paths leave accounting untouched.
pub fn apply_token_transfer(
    balance: i128,
    amount: i128,
    result: Result<bool, ()>,
) -> Result<i128, TokenError> {
    check_token_transfer_result(result)?;
    Ok(balance + amount)
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
    fn accepts_true_transfer_result() {
        assert_eq!(check_token_transfer_result(Ok(true)), Ok(()));
    }

    #[test]
    fn rejects_false_transfer_result() {
        assert_eq!(check_token_transfer_result(Ok(false)), Err(TokenError::TransferFailed));
    }

    #[test]
    fn rejects_malformed_transfer_result() {
        assert_eq!(check_token_transfer_result(Err(())), Err(TokenError::MalformedResponse));
    }

    #[test]
    fn commits_balance_on_success() {
        assert_eq!(apply_token_transfer(100, 25, Ok(true)), Ok(125));
    }

    #[test]
    fn leaves_balance_unchanged_on_false() {
        assert_eq!(apply_token_transfer(100, 25, Ok(false)), Err(TokenError::TransferFailed));
    }

    #[test]
    fn leaves_balance_unchanged_on_malformed() {
        assert_eq!(apply_token_transfer(100, 25, Err(())), Err(TokenError::MalformedResponse));
    }
}
