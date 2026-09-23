//! Initial scaffold for a stake withdrawal queue with cooldown enforcement (#920).
//! Models a queued unstake request that must sit through a cooldown window before
//! it becomes claimable, and expires if not claimed in time.
//! Follow-up work: wire into the live stake vault entrypoints/storage and ledger clock.

use soroban_sdk::{contracttype, Address, Env};

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

/// Configuration state holding the hard cap on total provider stake allocation.
#[derive(Clone)]
#[contracttype]
pub struct ProviderCapConfig {
    pub provider_cap: i128,
}

impl ProviderCapConfig {
    /// Validate and store the provider cap. The cap must be strictly positive so
    /// that a misconfigured zero/negative value cannot silently block all stake.
    pub fn new(provider_cap: i128) -> Self {
        if provider_cap <= 0 {
            panic!("provider cap must be positive");
        }
        Self { provider_cap }
    }

    /// Enforce the cap before any state mutation. Returns the accepted allocation
    /// on success, or rejects with an explanatory event when the cap is exceeded.
    pub fn enforce_allocation(&self, env: &Env, provider: &Address, current_stake: i128, attempted: i128) -> i128 {
        if attempted <= 0 {
            panic!("allocation must be positive");
        }
        let new_total = current_stake.saturating_add(attempted);
        if new_total > self.provider_cap {
            env.events().publish(
                (soroban_sdk::symbol_short!("cap_exceeded"), provider.clone()),
                (attempted, self.provider_cap, current_stake),
            );
            panic!("allocation exceeds provider cap");
        }
        env.events().publish(
            (soroban_sdk::symbol_short!("alloc_ok"), provider.clone()),
            (attempted, new_total),
        );
        new_total
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
    fn accepts_allocation_within_cap() {
        let env = Env::default();
        let cfg = ProviderCapConfig::new(1000);
        let total = cfg.enforce_allocation(&env, &owner(&env), 400, 500);
        assert_eq!(total, 900);
    }

    #[test]
    #[should_panic]
    fn rejects_allocation_over_cap() {
        let env = Env::default();
        let cfg = ProviderCapConfig::new(1000);
        cfg.enforce_allocation(&env, &owner(&env), 800, 500);
    }

    #[test]
    #[should_panic]
    fn rejects_non_positive_cap() {
        ProviderCapConfig::new(0);
    }
}
