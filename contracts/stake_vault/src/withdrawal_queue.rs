//! Initial scaffold for a stake withdrawal queue with cooldown enforcement (#920).
//! Models a queued unstake request that must sit through a cooldown window before
//! it becomes claimable, and expires if not claimed in time.
//! Follow-up work: wire into the live stake vault entrypoints/storage and ledger clock.
//!
//! #1036: partial withdraw requests are validated against user-level and
//! strategy-level minimum balance policies before any state is mutated.
//! Deposit retry protection (#1029): deposit transitions are guarded by a
//! per-transaction idempotency marker so a partially failed or retried write
//! cannot duplicate balances or rewards, and always leaves a consistent state.
//!
//! #1087: token transfer/approval return values are validated. Token
//! implementations that signal failure by returning `false` (or a malformed
//! value) instead of trapping are detected, and failure paths leave accounting
//! unchanged.

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

/// #1087: Result of validating a token operation's return value.
///
/// Token implementations differ in how they signal failure:
/// - Well-behaved tokens return `true` on success and `false` on failure.
/// - Some tokens return a malformed/empty value instead of a boolean.
/// - Others trap (panic) on failure.
///
/// Callers must treat anything other than an explicit `true` as a failure so
/// that a token which signals failure without trapping cannot silently leave
/// accounting mutated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub enum TokenOpOutcome {
    /// The token returned an explicit success value.
    Succeeded,
    /// The token returned `false` to signal failure without trapping.
    ReturnedFalse,
    /// The token returned a value that was not a boolean (malformed response).
    MalformedReturn,
}

impl TokenOpOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, TokenOpOutcome::Succeeded)
    }
}

/// #1087: Validate the raw return value of a token transfer/approval call.
///
/// `returned` is the optional boolean the token produced. `None` means the
/// token returned a malformed/empty value; `Some(false)` means it signalled
/// failure without trapping. Only `Some(true)` is treated as success.
///
/// This is a pure check so callers can validate the response *before* mutating
/// any accounting, guaranteeing failure paths leave balances unchanged.
pub fn check_token_return(returned: Option<bool>) -> TokenOpOutcome {
    match returned {
        Some(true) => TokenOpOutcome::Succeeded,
        Some(false) => TokenOpOutcome::ReturnedFalse,
        None => TokenOpOutcome::MalformedReturn,
    }
}

/// #1087: Apply a token operation and only commit accounting on success.
///
/// `op` performs the token transfer/approval and yields its raw return value.
/// `commit` applies the accounting mutation. `commit` is invoked *only* when
/// the token returned an explicit success value, so a token that signals
/// failure (false or malformed) leaves accounting unchanged. A token that
/// traps will abort the whole call before `commit` runs, which also leaves
/// accounting unchanged.
///
/// Returns the validated outcome so callers can branch on it.
pub fn apply_token_op<F, C>(op: F, commit: C) -> TokenOpOutcome
where
    F: FnOnce() -> Option<bool>,
    C: FnOnce(),
{
    let outcome = check_token_return(op());
    if outcome.is_success() {
        commit();
    }
    outcome
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
    fn expired_after_claim_window() {
        let env = Env::default();
        let req = WithdrawalRequest::new(owner(&env), 100, 0, 1000, 500);
        assert_eq!(req.status(2000), WithdrawalStatus::Expired);
        assert!(!req.is_claimable(2000));
    }

    #[test]
    fn token_return_true_is_success() {
        assert_eq!(check_token_return(Some(true)), TokenOpOutcome::Succeeded);
        assert!(check_token_return(Some(true)).is_success());
    }

    #[test]
    fn token_return_false_is_failure() {
        assert_eq!(check_token_return(Some(false)), TokenOpOutcome::ReturnedFalse);
        assert!(!check_token_return(Some(false)).is_success());
    }

    #[test]
    fn token_return_malformed_is_failure() {
        assert_eq!(check_token_return(None), TokenOpOutcome::MalformedReturn);
        assert!(!check_token_return(None).is_success());
    }

    #[test]
    fn commit_runs_only_on_success() {
        let mut committed = false;
        let outcome = apply_token_op(|| Some(true), || committed = true);
        assert_eq!(outcome, TokenOpOutcome::Succeeded);
        assert!(committed);
    }

    #[test]
    fn false_return_leaves_accounting_unchanged() {
        let mut committed = false;
        let outcome = apply_token_op(|| Some(false), || committed = true);
        assert_eq!(outcome, TokenOpOutcome::ReturnedFalse);
        assert!(!committed, "accounting must not change when token returns false");
    }

    #[test]
    fn malformed_return_leaves_accounting_unchanged() {
        let mut committed = false;
        let outcome = apply_token_op(|| None, || committed = true);
        assert_eq!(outcome, TokenOpOutcome::MalformedReturn);
        assert!(!committed, "accounting must not change on malformed token response");
    }
}
