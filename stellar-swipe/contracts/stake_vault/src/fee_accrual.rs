//! Deterministic fee accrual for the staking vault (Issue #1016).
//!
//! The vault distributes fee income to stakers in proportion to their share of
//! the pool.  To keep every `deposit`, `claim`, and `withdraw` on the *same*
//! accounting basis — regardless of the order accounts touch the vault within a
//! single ledger sequence — accrual uses a classic accumulator:
//!
//! ```text
//! acc_fee_per_share += floor(pool * ACC_PRECISION / total_shares)
//! account_accrued    = floor(shares * acc_fee_per_share / ACC_PRECISION) - fee_debt
//! ```
//!
//! ## Determinism & rounding rules
//!
//! * All division is integer floor division — never floating point, never
//!   round-half-up.  The same inputs always yield the same outputs.
//! * The dust lost to flooring each distribution is **not** discarded: it is
//!   accumulated in [`FeeAccrualState::carry`] and folded into the next
//!   deposit's distributable pool.  Total fees in == total fees claimable +
//!   carry, exactly, at every point in time.
//! * Deposits are rejected unless the supplied `ledger_seq` / `timestamp` are
//!   monotonically non-decreasing, so replays or out-of-order host calls cannot
//!   corrupt the accumulator.
//! * A deposit made while `total_shares == 0` is fully parked in `carry` and
//!   distributed to the first stakers that join.
//!
//! Every accrual update emits a precise accounting event (see
//! [`crate::fee_accrual::events`]).

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

/// Fixed-point precision for `acc_fee_per_share`.  `1e12` comfortably covers
/// realistic share/fee magnitudes for `i128` without overflow.
pub const ACC_PRECISION: i128 = 1_000_000_000_000;

/// Errors returned by the fee-accrual accounting functions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeeAccrualError {
    /// Deposit / share amount was zero or negative.
    InvalidAmount,
    /// `ledger_seq` or `timestamp` went backwards relative to the last update.
    NonMonotonicLedger,
    /// An intermediate calculation overflowed `i128`.
    Overflow,
    /// Attempted to remove more shares than the position holds.
    InsufficientShares,
    /// Nothing was accrued / available to claim.
    NothingToClaim,
}

// ── State ─────────────────────────────────────────────────────────────────────

/// Vault-wide fee-accrual accumulator.  One instance per vault.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeAccrualState {
    /// Cumulative fee per share, scaled by [`ACC_PRECISION`].
    pub acc_fee_per_share: i128,
    /// Sum of every position's `shares`.
    pub total_shares: i128,
    /// Dust from prior flooring + any deposits made with no shares present.
    /// Folded into the next deposit's distributable pool.
    pub carry: i128,
    /// Lifetime fees handed to this accumulator via `accrue_deposit`.
    pub total_deposited: i128,
    /// Lifetime fees actually credited to `acc_fee_per_share`.
    pub total_distributed: i128,
    /// Ledger sequence of the most recent accrual update (monotonic guard).
    pub last_ledger_seq: u32,
    /// Ledger timestamp of the most recent accrual update (monotonic guard).
    pub last_timestamp: u64,
    /// Monotonic counter incremented on every accrual update; identifies an
    /// accrual epoch in emitted events.
    pub epoch: u64,
}

impl FeeAccrualState {
    /// A fresh, empty accumulator.
    pub fn new() -> Self {
        FeeAccrualState {
            acc_fee_per_share: 0,
            total_shares: 0,
            carry: 0,
            total_deposited: 0,
            total_distributed: 0,
            last_ledger_seq: 0,
            last_timestamp: 0,
            epoch: 0,
        }
    }
}

impl Default for FeeAccrualState {
    fn default() -> Self {
        Self::new()
    }
}

/// A single staker's position in the fee-accrual accumulator.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakerFeePosition {
    pub staker: Address,
    /// Shares held.  Mirrors the staker's stake weight in the vault.
    pub shares: i128,
    /// `shares * acc_fee_per_share / ACC_PRECISION` captured at the last
    /// settlement — the baseline the next accrual is measured against.
    pub fee_debt: i128,
    /// Fees settled to the position but not yet claimed.
    pub realized: i128,
}

impl StakerFeePosition {
    pub fn new(staker: Address) -> Self {
        StakerFeePosition {
            staker,
            shares: 0,
            fee_debt: 0,
            realized: 0,
        }
    }
}

/// Summary of one accrual update, returned by [`accrue_deposit`] and emitted as
/// an event.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccrualUpdate {
    pub epoch: u64,
    /// Raw fee amount supplied to this deposit.
    pub deposited: i128,
    /// Amount actually credited to `acc_fee_per_share` this update.
    pub distributed: i128,
    /// Increment applied to `acc_fee_per_share` (scaled by `ACC_PRECISION`).
    pub per_share_delta: i128,
    /// Dust carried forward after this update.
    pub carry: i128,
    pub acc_fee_per_share: i128,
    pub total_shares: i128,
    pub ledger_seq: u32,
    pub timestamp: u64,
}

// ── Arithmetic helpers ────────────────────────────────────────────────────────

fn add(a: i128, b: i128) -> Result<i128, FeeAccrualError> {
    a.checked_add(b).ok_or(FeeAccrualError::Overflow)
}

fn sub(a: i128, b: i128) -> Result<i128, FeeAccrualError> {
    a.checked_sub(b).ok_or(FeeAccrualError::Overflow)
}

fn mul(a: i128, b: i128) -> Result<i128, FeeAccrualError> {
    a.checked_mul(b).ok_or(FeeAccrualError::Overflow)
}

/// `floor(shares * acc_fee_per_share / ACC_PRECISION)`.
fn accumulated_for(shares: i128, acc_fee_per_share: i128) -> Result<i128, FeeAccrualError> {
    let scaled = mul(shares, acc_fee_per_share)?;
    Ok(scaled / ACC_PRECISION)
}

// ── Core accounting ───────────────────────────────────────────────────────────

/// Credit `amount` of fee income to the accumulator.
///
/// `ledger_seq` / `timestamp` come from `env.ledger()` and must never move
/// backwards between calls — this is the determinism guard that makes every
/// subsequent `settle` reproducible.
///
/// # Errors
/// * [`FeeAccrualError::InvalidAmount`] — `amount <= 0`.
/// * [`FeeAccrualError::NonMonotonicLedger`] — ledger metadata went backwards.
/// * [`FeeAccrualError::Overflow`] — an intermediate multiplication overflowed.
pub fn accrue_deposit(
    env: &Env,
    state: &mut FeeAccrualState,
    amount: i128,
    ledger_seq: u32,
    timestamp: u64,
) -> Result<AccrualUpdate, FeeAccrualError> {
    if amount <= 0 {
        return Err(FeeAccrualError::InvalidAmount);
    }
    if ledger_seq < state.last_ledger_seq || timestamp < state.last_timestamp {
        return Err(FeeAccrualError::NonMonotonicLedger);
    }

    let pool = add(amount, state.carry)?;

    let (per_share_delta, distributed) = if state.total_shares == 0 {
        // No shares yet — park the whole pool until stakers arrive.
        (0i128, 0i128)
    } else {
        let delta = mul(pool, ACC_PRECISION)? / state.total_shares;
        // Only the portion that divides evenly across shares is "distributed";
        // the rest stays as carry so nothing is ever double-counted or lost.
        let distributed = mul(delta, state.total_shares)? / ACC_PRECISION;
        (delta, distributed)
    };

    state.acc_fee_per_share = add(state.acc_fee_per_share, per_share_delta)?;
    state.carry = sub(pool, distributed)?;
    state.total_deposited = add(state.total_deposited, amount)?;
    state.total_distributed = add(state.total_distributed, distributed)?;
    state.last_ledger_seq = ledger_seq;
    state.last_timestamp = timestamp;
    state.epoch = state.epoch.saturating_add(1);

    let update = AccrualUpdate {
        epoch: state.epoch,
        deposited: amount,
        distributed,
        per_share_delta,
        carry: state.carry,
        acc_fee_per_share: state.acc_fee_per_share,
        total_shares: state.total_shares,
        ledger_seq,
        timestamp,
    };
    events::emit_accrual(env, &update);
    Ok(update)
}

/// Settle any outstanding accrual for `position` against the current
/// accumulator, moving it into `position.realized`.  Returns the amount just
/// settled (always `>= 0`).
///
/// Idempotent: calling twice in a row settles zero the second time.
pub fn settle(
    state: &FeeAccrualState,
    position: &mut StakerFeePosition,
) -> Result<i128, FeeAccrualError> {
    let accumulated = accumulated_for(position.shares, state.acc_fee_per_share)?;
    let newly = sub(accumulated, position.fee_debt)?;
    // `newly` can only be negative if shares were mutated without settling
    // first; callers below always settle before mutating, so treat as 0.
    let newly = if newly < 0 { 0 } else { newly };
    position.realized = add(position.realized, newly)?;
    position.fee_debt = accumulated;
    Ok(newly)
}

/// Add `amount` shares to `position`, settling first so the new shares do not
/// retroactively earn past fees.
pub fn add_shares(
    env: &Env,
    state: &mut FeeAccrualState,
    position: &mut StakerFeePosition,
    amount: i128,
) -> Result<(), FeeAccrualError> {
    if amount <= 0 {
        return Err(FeeAccrualError::InvalidAmount);
    }
    settle(state, position)?;
    position.shares = add(position.shares, amount)?;
    state.total_shares = add(state.total_shares, amount)?;
    position.fee_debt = accumulated_for(position.shares, state.acc_fee_per_share)?;
    events::emit_shares_changed(env, &position.staker, position.shares, state.total_shares);
    Ok(())
}

/// Remove `amount` shares from `position`, settling first so already-earned
/// fees are preserved in `position.realized`.
pub fn remove_shares(
    env: &Env,
    state: &mut FeeAccrualState,
    position: &mut StakerFeePosition,
    amount: i128,
) -> Result<(), FeeAccrualError> {
    if amount <= 0 {
        return Err(FeeAccrualError::InvalidAmount);
    }
    if amount > position.shares {
        return Err(FeeAccrualError::InsufficientShares);
    }
    settle(state, position)?;
    position.shares = sub(position.shares, amount)?;
    state.total_shares = sub(state.total_shares, amount)?;
    position.fee_debt = accumulated_for(position.shares, state.acc_fee_per_share)?;
    events::emit_shares_changed(env, &position.staker, position.shares, state.total_shares);
    Ok(())
}

/// Settle and zero out `position.realized`, returning the claimed amount.
///
/// # Errors
/// * [`FeeAccrualError::NothingToClaim`] — nothing is available after settling.
pub fn claim(
    env: &Env,
    state: &FeeAccrualState,
    position: &mut StakerFeePosition,
) -> Result<i128, FeeAccrualError> {
    settle(state, position)?;
    let claimed = position.realized;
    if claimed <= 0 {
        return Err(FeeAccrualError::NothingToClaim);
    }
    position.realized = 0;
    events::emit_claim(env, &position.staker, claimed);
    Ok(claimed)
}

/// Read-only: fees `position` could claim right now (realized + unsettled),
/// without mutating anything.
pub fn pending(
    state: &FeeAccrualState,
    position: &StakerFeePosition,
) -> Result<i128, FeeAccrualError> {
    let accumulated = accumulated_for(position.shares, state.acc_fee_per_share)?;
    let unsettled = sub(accumulated, position.fee_debt)?;
    let unsettled = if unsettled < 0 { 0 } else { unsettled };
    add(position.realized, unsettled)
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Event emission for fee accrual.  Follows the two-topic convention:
/// `topics = ("stake_vault", <event>)`, body = a `#[contracttype]` struct.
pub mod events {
    use super::{AccrualUpdate, Env, Symbol};
    use soroban_sdk::{contracttype, symbol_short, Address};

    fn contract_topic(env: &Env) -> Symbol {
        Symbol::new(env, "stake_vault")
    }

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct EvtFeeAccrued {
        pub schema_version: u32,
        pub epoch: u64,
        pub deposited: i128,
        pub distributed: i128,
        pub per_share_delta: i128,
        pub carry: i128,
        pub acc_fee_per_share: i128,
        pub total_shares: i128,
        pub ledger_seq: u32,
        pub timestamp: u64,
    }

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct EvtFeeSharesChanged {
        pub schema_version: u32,
        pub staker: Address,
        pub shares: i128,
        pub total_shares: i128,
    }

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct EvtFeeClaimed {
        pub schema_version: u32,
        pub staker: Address,
        pub amount: i128,
    }

    pub(super) fn emit_accrual(env: &Env, u: &AccrualUpdate) {
        env.events().publish(
            (contract_topic(env), symbol_short!("fee_accr")),
            EvtFeeAccrued {
                schema_version: 1,
                epoch: u.epoch,
                deposited: u.deposited,
                distributed: u.distributed,
                per_share_delta: u.per_share_delta,
                carry: u.carry,
                acc_fee_per_share: u.acc_fee_per_share,
                total_shares: u.total_shares,
                ledger_seq: u.ledger_seq,
                timestamp: u.timestamp,
            },
        );
    }

    pub(super) fn emit_shares_changed(
        env: &Env,
        staker: &Address,
        shares: i128,
        total_shares: i128,
    ) {
        env.events().publish(
            (contract_topic(env), symbol_short!("fee_shrs")),
            EvtFeeSharesChanged {
                schema_version: 1,
                staker: staker.clone(),
                shares,
                total_shares,
            },
        );
    }

    pub(super) fn emit_claim(env: &Env, staker: &Address, amount: i128) {
        env.events().publish(
            (contract_topic(env), symbol_short!("fee_clm")),
            EvtFeeClaimed {
                schema_version: 1,
                staker: staker.clone(),
                amount,
            },
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    fn setup() -> (Env, FeeAccrualState) {
        (Env::default(), FeeAccrualState::new())
    }

    /// Total fees in must always equal claimable + carry.
    fn assert_conservation(state: &FeeAccrualState, positions: &[&StakerFeePosition]) {
        let mut claimable = 0i128;
        for p in positions {
            claimable += pending(state, p).unwrap();
        }
        assert_eq!(
            state.total_deposited,
            claimable + state.carry,
            "fee conservation violated"
        );
    }

    #[test]
    fn even_split_two_equal_stakers() {
        let (env, mut state) = setup();
        let a = StakerFeePosition::new(Address::generate(&env));
        let b = StakerFeePosition::new(Address::generate(&env));
        let (mut a, mut b) = (a, b);

        add_shares(&env, &mut state, &mut a, 100).unwrap();
        add_shares(&env, &mut state, &mut b, 100).unwrap();

        // Deposit 1000 over 200 shares => 5 per share.
        let u = accrue_deposit(&env, &mut state, 1_000, 1, 100).unwrap();
        assert_eq!(u.distributed, 1_000);
        assert_eq!(u.carry, 0);

        assert_eq!(pending(&state, &a).unwrap(), 500);
        assert_eq!(pending(&state, &b).unwrap(), 500);
        assert_conservation(&state, &[&a, &b]);
    }

    #[test]
    fn deterministic_regardless_of_settle_order() {
        let (env, mut state_1) = setup();
        let mut state_2 = FeeAccrualState::new();

        let addr_a = Address::generate(&env);
        let addr_b = Address::generate(&env);

        let mut a1 = StakerFeePosition::new(addr_a.clone());
        let mut b1 = StakerFeePosition::new(addr_b.clone());
        let mut a2 = StakerFeePosition::new(addr_a);
        let mut b2 = StakerFeePosition::new(addr_b);

        for (s, a, b) in [
            (&mut state_1, &mut a1, &mut b1),
            (&mut state_2, &mut a2, &mut b2),
        ] {
            add_shares(&env, s, a, 30).unwrap();
            add_shares(&env, s, b, 70).unwrap();
            accrue_deposit(&env, s, 777, 5, 500).unwrap();
        }

        // state_1: settle A then B.  state_2: settle B then A.
        settle(&state_1, &mut a1).unwrap();
        settle(&state_1, &mut b1).unwrap();
        settle(&state_2, &mut b2).unwrap();
        settle(&state_2, &mut a2).unwrap();

        assert_eq!(a1.realized, a2.realized);
        assert_eq!(b1.realized, b2.realized);
        assert_eq!(state_1, state_2);
    }

    #[test]
    fn dust_is_carried_not_lost() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        let mut c = StakerFeePosition::new(Address::generate(&env));

        add_shares(&env, &mut state, &mut a, 1).unwrap();
        add_shares(&env, &mut state, &mut b, 1).unwrap();
        add_shares(&env, &mut state, &mut c, 1).unwrap();

        // 10 over 3 shares — not evenly divisible.
        let u = accrue_deposit(&env, &mut state, 10, 1, 1).unwrap();
        assert_eq!(u.distributed, 9);
        assert_eq!(u.carry, 1);

        // Next deposit folds the carried dust back in: pool = 5 + 1 = 6 => 2/share.
        let u2 = accrue_deposit(&env, &mut state, 5, 2, 2).unwrap();
        assert_eq!(u2.distributed, 6);
        assert_eq!(u2.carry, 0);

        assert_conservation(&state, &[&a, &b, &c]);
        assert_eq!(state.total_distributed, 15);
    }

    #[test]
    fn late_joiner_earns_nothing_from_prior_deposit() {
        let (env, mut state) = setup();
        let mut early = StakerFeePosition::new(Address::generate(&env));
        let mut late = StakerFeePosition::new(Address::generate(&env));

        add_shares(&env, &mut state, &mut early, 100).unwrap();
        accrue_deposit(&env, &mut state, 400, 1, 10).unwrap();

        add_shares(&env, &mut state, &mut late, 100).unwrap();
        assert_eq!(pending(&state, &late).unwrap(), 0);
        assert_eq!(pending(&state, &early).unwrap(), 400);

        // Second deposit splits evenly now.
        accrue_deposit(&env, &mut state, 200, 2, 20).unwrap();
        assert_eq!(pending(&state, &early).unwrap(), 500);
        assert_eq!(pending(&state, &late).unwrap(), 100);
        assert_conservation(&state, &[&early, &late]);
    }

    #[test]
    fn deposit_with_no_shares_is_parked_then_distributed() {
        let (env, mut state) = setup();
        let u = accrue_deposit(&env, &mut state, 100, 1, 1).unwrap();
        assert_eq!(u.distributed, 0);
        assert_eq!(state.carry, 100);

        let mut first = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut first, 50).unwrap();
        // Parked fees only move on the next deposit.
        accrue_deposit(&env, &mut state, 50, 2, 2).unwrap();
        assert_eq!(pending(&state, &first).unwrap(), 150);
        assert_conservation(&state, &[&first]);
    }

    #[test]
    fn claim_zeroes_realized_and_is_not_repeatable() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 10).unwrap();
        accrue_deposit(&env, &mut state, 100, 1, 1).unwrap();

        assert_eq!(claim(&env, &state, &mut a).unwrap(), 100);
        assert_eq!(
            claim(&env, &state, &mut a),
            Err(FeeAccrualError::NothingToClaim)
        );

        // A later deposit accrues fresh, claimable fees.
        accrue_deposit(&env, &mut state, 50, 2, 2).unwrap();
        assert_eq!(claim(&env, &state, &mut a).unwrap(), 50);
    }

    #[test]
    fn remove_shares_preserves_earned_fees() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 100).unwrap();
        add_shares(&env, &mut state, &mut b, 100).unwrap();
        accrue_deposit(&env, &mut state, 1_000, 1, 1).unwrap();

        remove_shares(&env, &mut state, &mut a, 100).unwrap();
        assert_eq!(a.shares, 0);
        assert_eq!(a.realized, 500);

        // b now owns the whole pool for subsequent deposits.
        accrue_deposit(&env, &mut state, 300, 2, 2).unwrap();
        assert_eq!(pending(&state, &b).unwrap(), 800);
        assert_eq!(pending(&state, &a).unwrap(), 500);
    }

    #[test]
    fn rejects_bad_input() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        assert_eq!(
            accrue_deposit(&env, &mut state, 0, 1, 1),
            Err(FeeAccrualError::InvalidAmount)
        );
        assert_eq!(
            add_shares(&env, &mut state, &mut a, -5),
            Err(FeeAccrualError::InvalidAmount)
        );
        add_shares(&env, &mut state, &mut a, 10).unwrap();
        assert_eq!(
            remove_shares(&env, &mut state, &mut a, 11),
            Err(FeeAccrualError::InsufficientShares)
        );
    }

    #[test]
    fn rejects_non_monotonic_ledger() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 10).unwrap();
        accrue_deposit(&env, &mut state, 100, 10, 100).unwrap();
        assert_eq!(
            accrue_deposit(&env, &mut state, 100, 9, 100),
            Err(FeeAccrualError::NonMonotonicLedger)
        );
        assert_eq!(
            accrue_deposit(&env, &mut state, 100, 10, 99),
            Err(FeeAccrualError::NonMonotonicLedger)
        );
        // Equal ledger_seq / timestamp is allowed (same-sequence interactions).
        accrue_deposit(&env, &mut state, 100, 10, 100).unwrap();
    }

    #[test]
    fn manual_scenario_matches_on_chain() {
        // Hand-computed reference scenario.
        //   t0: A=200 shares, B=300 shares  (total 500)
        //   t1: deposit 5_000  -> 10 per share ; A=2_000 B=3_000
        //   t2: C joins with 500 shares (total 1_000)
        //   t3: deposit 7_000  -> 7 per share ; A+1_400 B+2_100 C+3_500
        //   final: A=3_400 B=5_100 C=3_500 ; sum 12_000 == deposited
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        let mut c = StakerFeePosition::new(Address::generate(&env));

        add_shares(&env, &mut state, &mut a, 200).unwrap();
        add_shares(&env, &mut state, &mut b, 300).unwrap();
        accrue_deposit(&env, &mut state, 5_000, 1, 1).unwrap();
        add_shares(&env, &mut state, &mut c, 500).unwrap();
        accrue_deposit(&env, &mut state, 7_000, 2, 2).unwrap();

        assert_eq!(pending(&state, &a).unwrap(), 3_400);
        assert_eq!(pending(&state, &b).unwrap(), 5_100);
        assert_eq!(pending(&state, &c).unwrap(), 3_500);
        assert_eq!(state.total_deposited, 12_000);
        assert_conservation(&state, &[&a, &b, &c]);
    }
}
