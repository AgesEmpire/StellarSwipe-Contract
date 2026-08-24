//! Initial scaffold for a stake withdrawal queue with cooldown enforcement (#920).
//! Models a queued unstake request that must sit through a cooldown window before
//! it becomes claimable, and expires if not claimed in time.
//! Follow-up work: wire into the live stake vault entrypoints/storage and ledger clock.

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
}
