//! Initial scaffold for a stake withdrawal queue with cooldown enforcement (#920).
//! Models a queued unstake request that must sit through a cooldown window before
//! it becomes claimable, and expires if not claimed in time.
//! Follow-up work: wire into the live stake vault entrypoints/storage and ledger clock.
//!
//! #1036: partial withdraw requests are validated against user-level and
//! strategy-level minimum balance policies before any state is mutated.

use soroban_sdk::{contracttype, Address};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub enum WithdrawalStatus {
    Queued,
    Active,
    Expired,
    Claimed,
}

#[derive(Clone)]
#[contracttype]
pub struct WithdrawalRequest {
    pub owner: Address,
    pub amount: i128,
    pub requested_at: u64,
    pub cooldown_seconds: u64,
    pub expiry_seconds: u64,
}

impl WithdrawalRequest {
    pub fn new(owner: Address, amount: i128, requested_at: u64, cooldown_seconds: u64, expiry_seconds: u64) -> Self {
        Self { owner, amount, requested_at, cooldown_seconds, expiry_seconds }
    }

    /// Status is determined purely from elapsed time relative to requested_at,
    /// so it can be computed on read without extra state transitions.
    pub fn status(&self, now: u64) -> WithdrawalStatus {
        let elapsed = now.saturating_sub(self.requested_at);
        if elapsed < self.cooldown_seconds {
            WithdrawalStatus::Queued
        } else if elapsed < self.cooldown_seconds + self.expiry_seconds {
            WithdrawalStatus::Active
        } else {
            WithdrawalStatus::Expired
        }
    }

    pub fn is_claimable(&self, now: u64) -> bool {
        self.status(now) == WithdrawalStatus::Active
    }
}

/// Minimum balance policies enforced on partial withdraws (#1036).
/// `user_minimum` is the reserve the owner must keep in their position;
/// `strategy_minimum` is the reserve the strategy must keep after the withdraw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub struct MinimumBalancePolicy {
    pub user_minimum: i128,
    pub strategy_minimum: i128,
}

impl MinimumBalancePolicy {
    pub fn new(user_minimum: i128, strategy_minimum: i128) -> Self {
        Self { user_minimum, strategy_minimum }
    }
}

/// Reasons a partial withdraw request can be rejected before state mutation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub enum WithdrawRejection {
    NonPositiveAmount,
    ExceedsUserBalance,
    BelowUserMinimum,
    BelowStrategyMinimum,
}

/// Outcome of validating a partial withdraw against the active policy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub enum WithdrawCheck {
    Accepted,
    Rejected(WithdrawRejection),
}

impl WithdrawCheck {
    pub fn is_accepted(&self) -> bool {
        matches!(self, WithdrawCheck::Accepted)
    }
}

/// Validate a partial withdraw request against user-level and strategy-level
/// minimum balance policies. Pure check: no balances are mutated here, so
/// callers can reject invalid withdrawals before touching state.
///
/// `user_balance` is the owner's current position; `strategy_balance` is the
/// strategy's current total. `amount` is the requested partial withdraw.
pub fn check_partial_withdraw(
    amount: i128,
    user_balance: i128,
    strategy_balance: i128,
    policy: MinimumBalancePolicy,
) -> WithdrawCheck {
    if amount <= 0 {
        return WithdrawCheck::Rejected(WithdrawRejection::NonPositiveAmount);
    }
    if amount > user_balance {
        return WithdrawCheck::Rejected(WithdrawRejection::ExceedsUserBalance);
    }
    if user_balance - amount < policy.user_minimum {
        return WithdrawCheck::Rejected(WithdrawRejection::BelowUserMinimum);
    }
    if strategy_balance - amount < policy.strategy_minimum {
        return WithdrawCheck::Rejected(WithdrawRejection::BelowStrategyMinimum);
    }
    WithdrawCheck::Accepted
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn owner(env: &Env) -> Address {
        Address::generate(env)
    }

    #[test]
    fn queued_before_cooldown_elapses() {
        let env = Env::default();
        let req = WithdrawalRequest::new(owner(&env), 100, 0, 1000, 500);
        assert_eq!(req.status(500), WithdrawalStatus::Queued);
        assert!(!req.is_claimable(500));
    }

    #[test]
    fn active_within_claim_window() {
        let env = Env::default();
        let req = WithdrawalRequest::new(owner(&env), 100, 0, 1000, 500);
        assert_eq!(req.status(1200), WithdrawalStatus::Active);
        assert!(req.is_claimable(1200));
    }

    #[test]
    fn expires_after_claim_window() {
        let env = Env::default();
        let req = WithdrawalRequest::new(owner(&env), 100, 0, 1000, 500);
        assert_eq!(req.status(1600), WithdrawalStatus::Expired);
        assert!(!req.is_claimable(1600));
    }

    #[test]
    fn accepts_withdraw_above_both_minimums() {
        let policy = MinimumBalancePolicy::new(100, 500);
        assert_eq!(check_partial_withdraw(200, 1000, 5000, policy), WithdrawCheck::Accepted);
    }

    #[test]
    fn rejects_non_positive_amount() {
        let policy = MinimumBalancePolicy::new(100, 500);
        assert_eq!(
            check_partial_withdraw(0, 1000, 5000, policy),
            WithdrawCheck::Rejected(WithdrawRejection::NonPositiveAmount)
        );
    }

    #[test]
    fn rejects_amount_exceeding_user_balance() {
        let policy = MinimumBalancePolicy::new(100, 500);
        assert_eq!(
            check_partial_withdraw(2000, 1000, 5000, policy),
            WithdrawCheck::Rejected(WithdrawRejection::ExceedsUserBalance)
        );
    }

    #[test]
    fn rejects_withdraw_breaching_user_minimum() {
        let policy = MinimumBalancePolicy::new(900, 500);
        assert_eq!(
            check_partial_withdraw(200, 1000, 5000, policy),
            WithdrawCheck::Rejected(WithdrawRejection::BelowUserMinimum)
        );
    }

    #[test]
    fn rejects_withdraw_breaching_strategy_minimum() {
        let policy = MinimumBalancePolicy::new(100, 4900);
        assert_eq!(
            check_partial_withdraw(200, 1000, 5000, policy),
            WithdrawCheck::Rejected(WithdrawRejection::BelowStrategyMinimum)
        );
    }
}
