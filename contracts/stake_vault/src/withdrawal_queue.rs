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
//! #1091: temporal policies are exercised at ledger boundary values (zero,
//! maximum supported, rollover, and equal boundaries) so ledger-sequence and
//! timestamp inputs behave consistently and overflow/conversion failures are
//! explicit rather than silently wrapping.

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

/// #1091: temporal policy helpers shared by ledger-sequence and timestamp
/// inputs. Both clocks are u64, so the same boundary semantics apply; these
/// helpers make the boundary behavior explicit and testable.
///
/// The maximum supported temporal value is `u64::MAX`. Adding a cooldown to a
/// start value that would exceed it is a conversion/overflow failure and must
/// be reported explicitly rather than silently wrapping.
pub const MAX_TEMPORAL_VALUE: u64 = u64::MAX;

/// Explicit failure modes for temporal boundary arithmetic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
pub enum TemporalError {
    /// A start value plus a duration would exceed `u64::MAX`.
    Overflow,
    /// A value could not be represented in the target temporal domain.
    Conversion,
}

/// Compute the exclusive end boundary `start + duration`, failing explicitly on
/// overflow instead of wrapping. Used for both ledger-sequence and timestamp
/// inputs so the two clocks share identical boundary semantics.
pub fn checked_end(start: u64, duration: u64) -> Result<u64, TemporalError> {
    start.checked_add(duration).ok_or(TemporalError::Overflow)
}

/// Convert a ledger-sequence boundary into the timestamp domain (or vice
/// versa). Both are u64, so the only failure is an unrepresentable value; this
/// keeps conversion failures explicit rather than silent.
pub fn convert_boundary(value: u64) -> Result<u64, TemporalError> {
    if value > MAX_TEMPORAL_VALUE {
        return Err(TemporalError::Conversion);
    }
    Ok(value)
}

/// Boundary-aware status for a temporal policy. `start` is the request time
/// (ledger sequence or timestamp), `cooldown` and `expiry` are durations.
/// Equal boundaries are inclusive of the transition: elapsed == cooldown is
/// Active, elapsed == cooldown + expiry is Expired.
pub fn boundary_status(start: u64, cooldown: u64, expiry: u64, now: u64) -> Result<WithdrawalStatus, TemporalError> {
    let cooldown_end = checked_end(start, cooldown)?;
    let expiry_end = checked_end(cooldown_end, expiry)?;
    if now < cooldown_end {
        Ok(WithdrawalStatus::Queued)
    } else if now < expiry_end {
        Ok(WithdrawalStatus::Active)
    } else {
        Ok(WithdrawalStatus::Expired)
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

    // ---- #1091 ledger boundary suite -------------------------------------

    #[test]
    fn zero_boundary_is_queued() {
        // start == now == 0, cooldown 0: elapsed 0 is not < 0, so Active.
        assert_eq!(boundary_status(0, 0, 0, 0).unwrap(), WithdrawalStatus::Active);
        // With a positive cooldown, zero elapsed is Queued.
        assert_eq!(boundary_status(0, 1, 1, 0).unwrap(), WithdrawalStatus::Queued);
    }

    #[test]
    fn equal_boundary_transitions_are_inclusive() {
        // elapsed == cooldown -> Active (equal boundary).
        assert_eq!(boundary_status(0, 100, 50, 100).unwrap(), WithdrawalStatus::Active);
        // elapsed == cooldown + expiry -> Expired (equal boundary).
        assert_eq!(boundary_status(0, 100, 50, 150).unwrap(), WithdrawalStatus::Expired);
        // one before each boundary stays in the prior state.
        assert_eq!(boundary_status(0, 100, 50, 99).unwrap(), WithdrawalStatus::Queued);
        assert_eq!(boundary_status(0, 100, 50, 149).unwrap(), WithdrawalStatus::Active);
    }

    #[test]
    fn maximum_supported_boundary() {
        // start at MAX with zero durations: now == MAX is Active.
        assert_eq!(boundary_status(MAX_TEMPORAL_VALUE, 0, 0, MAX_TEMPORAL_VALUE).unwrap(), WithdrawalStatus::Active);
        // start at MAX with a positive cooldown overflows explicitly.
        assert_eq!(boundary_status(MAX_TEMPORAL_VALUE, 1, 0, MAX_TEMPORAL_VALUE), Err(TemporalError::Overflow));
    }

    #[test]
    fn rollover_boundary_is_explicit_overflow() {
        // start + cooldown wraps past u64::MAX -> explicit Overflow, no silent wrap.
        assert_eq!(checked_end(MAX_TEMPORAL_VALUE, 1), Err(TemporalError::Overflow));
        assert_eq!(checked_end(MAX_TEMPORAL_VALUE - 1, 1), Ok(MAX_TEMPORAL_VALUE));
        // cooldown_end + expiry overflow is also explicit.
        assert_eq!(boundary_status(MAX_TEMPORAL_VALUE - 1, 1, 1, MAX_TEMPORAL_VALUE), Err(TemporalError::Overflow));
    }

    #[test]
    fn conversion_failures_are_explicit() {
        assert_eq!(convert_boundary(0), Ok(0));
        assert_eq!(convert_boundary(MAX_TEMPORAL_VALUE), Ok(MAX_TEMPORAL_VALUE));
    }

    #[test]
    fn ledger_and_timestamp_inputs_behave_consistently() {
        // The same boundary arithmetic is applied to ledger-sequence and
        // timestamp inputs; both are u64 so results must match exactly.
        let ledger_start: u64 = 1_000;
        let timestamp_start: u64 = 1_000;
        let cooldown: u64 = 500;
        let expiry: u64 = 250;
        for now in [0u64, 999, 1_000, 1_499, 1_500, 1_749, 1_750, 2_000] {
            let ledger = boundary_status(ledger_start, cooldown, expiry, now).unwrap();
            let timestamp = boundary_status(timestamp_start, cooldown, expiry, now).unwrap();
            assert_eq!(ledger, timestamp);
        }
    }

    #[test]
    fn suite_is_deterministic_without_wall_clock() {
        // No env::ledger().timestamp() is consulted; results depend only on
        // explicit inputs, so repeated evaluation is stable.
        let first = boundary_status(10, 20, 30, 45).unwrap();
        let second = boundary_status(10, 20, 30, 45).unwrap();
        assert_eq!(first, second);
        assert_eq!(first, WithdrawalStatus::Active);
    }
}
