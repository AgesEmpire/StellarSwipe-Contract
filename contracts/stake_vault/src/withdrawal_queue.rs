//! Initial scaffold for a stake withdrawal queue with cooldown enforcement (#920).
//! Models a queued unstake request that must sit through a cooldown window before
//! it becomes claimable, and expires if not claimed in time.
//! Follow-up work: wire into the live stake vault entrypoints/storage and ledger clock.
//!
//! Deposit retry protection (#1029): deposit transitions are guarded by a
//! per-transaction idempotency marker so a partially failed or retried write
//! cannot duplicate balances or rewards, and always leaves a consistent state.

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

/// Outcome of a guarded deposit transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub enum DepositOutcome {
    /// The deposit was applied for the first time.
    Applied,
    /// The deposit was already applied for this transaction marker; no-op.
    AlreadyApplied,
}

/// Tracks the last applied deposit marker so retried or partially failed
/// deposit writes can be detected and skipped instead of duplicating balances
/// or rewards.
#[derive(Clone)]
#[contracttype]
pub struct DepositGuard {
    pub last_marker: u64,
    pub applied: bool,
}

impl DepositGuard {
    pub fn new() -> Self {
        Self { last_marker: 0, applied: false }
    }

    /// Returns true when `marker` has already been applied, meaning the caller
    /// is retrying a deposit that previously succeeded and must not be applied
    /// again.
    pub fn is_retry(&self, marker: u64) -> bool {
        self.applied && self.last_marker == marker
    }

    /// Guard a deposit transition. If the marker was already applied the write
    /// is skipped (idempotent); otherwise the marker is recorded so a later
    /// retry is detected. Returns the outcome so callers can branch on it.
    pub fn apply(&mut self, marker: u64) -> DepositOutcome {
        if self.is_retry(marker) {
            return DepositOutcome::AlreadyApplied;
        }
        self.last_marker = marker;
        self.applied = true;
        DepositOutcome::Applied
    }

    /// Reset the guard after a failed deposit so the contract is left in a
    /// consistent state and the transition can be safely retried.
    pub fn rollback(&mut self) {
        self.last_marker = 0;
        self.applied = false;
    }
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
    fn first_deposit_is_applied() {
        let mut guard = DepositGuard::new();
        assert_eq!(guard.apply(7), DepositOutcome::Applied);
        assert!(!guard.is_retry(7));
    }

    #[test]
    fn repeated_deposit_marker_is_detected_and_skipped() {
        let mut guard = DepositGuard::new();
        assert_eq!(guard.apply(7), DepositOutcome::Applied);
        assert!(guard.is_retry(7));
        assert_eq!(guard.apply(7), DepositOutcome::AlreadyApplied);
    }

    #[test]
    fn rollback_allows_safe_retry() {
        let mut guard = DepositGuard::new();
        assert_eq!(guard.apply(7), DepositOutcome::Applied);
        guard.rollback();
        assert!(!guard.is_retry(7));
        assert_eq!(guard.apply(7), DepositOutcome::Applied);
    }
}
