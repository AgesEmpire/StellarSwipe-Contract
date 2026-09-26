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
//! * The remainder of each distribution is **not** discarded.  It is kept
//!   *exactly*, in scaled units: whole units in [`FeeAccrualState::carry`] and
//!   the sub-unit part in [`FeeAccrualState::dust_scaled`].  Both are folded
//!   into the next deposit's pool, so `total_deposited == total_distributed +
//!   carry` holds exactly and the fractional part is never counted twice.
//! * Positions are paid `floor` of their exact entitlement, and share-change
//!   checkpoints record `fee_debt` rounded **up**, so a position can never
//!   claim more than it was credited.  Solvency always holds:
//!   `sum(pending) + carry <= total_deposited`.  Equality holds whenever
//!   amounts divide evenly; otherwise the gap is bounded sub-unit rounding
//!   dust that stays in the vault (it is never paid out twice).
//! * Deposits are rejected unless the supplied `ledger_seq` / `timestamp` are
//!   monotonically non-decreasing, so replays or out-of-order host calls cannot
//!   corrupt the accumulator.
//! * A deposit made while `total_shares == 0` is fully parked in `carry` and
//!   distributed to the first stakers that join.
//!
//! Every accrual update emits a precise accounting event (see
//! [`crate::fee_accrual::events`]).
//!
//! ## Slashing and accrued rewards (Issue #1205)
//!
//! A slash reduces a position's **principal (shares) only**.  Rewards the
//! position earned before the slash belong to the staker and are never
//! confiscated by it.  [`apply_slash`] enforces this with a fixed ordering:
//!
//! 1. **Settle** at the pre-slash share count: everything accrued up to the
//!    current `acc_fee_per_share` moves into `position.realized`.
//! 2. **Reduce** `position.shares` and `state.total_shares` by the slashed
//!    amount (clamped to the position's shares — a full slash leaves 0).
//! 3. **Re-checkpoint** `fee_debt` at the post-slash share count, so later
//!    accrual is measured only against the shares that remain.
//!
//! Because `total_shares` shrinks in the same step, every later deposit is
//! divided across the surviving shares only: the slashed shares neither keep
//! earning nor dilute other stakers.  The invariants
//! `sum(position.shares) == state.total_shares` and
//! `total_deposited == sum(pending) + carry` hold before and after a slash.

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
    /// Whole units not yet credited: deposits made with no shares present,
    /// plus whole units of division remainder.  Folded into the next
    /// deposit's distributable pool.
    pub carry: i128,
    /// Sub-unit division remainder, scaled by [`ACC_PRECISION`]
    /// (`0 <= dust_scaled < ACC_PRECISION`).  Folded into the next deposit so
    /// fractional value is neither lost nor credited twice.
    pub dust_scaled: i128,
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
            dust_scaled: 0,
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

/// Outcome of [`apply_slash`], also emitted as an event for auditing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashSettlement {
    /// Rewards settled into `realized` at the pre-slash share count.
    pub settled: i128,
    /// Shares actually removed (the request clamped to the position's shares).
    pub slashed_shares: i128,
    pub shares_before: i128,
    pub shares_after: i128,
    /// `position.realized` after the slash — preserved, never reduced.
    pub realized: i128,
    /// Vault-wide `total_shares` after the slash.
    pub total_shares: i128,
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

/// `ceil(shares * acc_fee_per_share / ACC_PRECISION)` — the `fee_debt`
/// checkpoint after a share change.  Rounding the debt up (while payouts round
/// down) guarantees a position never claims value it was not credited.
fn debt_checkpoint(shares: i128, acc_fee_per_share: i128) -> Result<i128, FeeAccrualError> {
    let scaled = mul(shares, acc_fee_per_share)?;
    let floor = scaled / ACC_PRECISION;
    if scaled % ACC_PRECISION == 0 {
        Ok(floor)
    } else {
        add(floor, 1)
    }
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
    let pool_scaled = add(mul(pool, ACC_PRECISION)?, state.dust_scaled)?;

    let (per_share_delta, remainder_scaled) = if state.total_shares == 0 {
        // No shares yet — park the whole pool until stakers arrive.
        (0i128, pool_scaled)
    } else {
        let delta = pool_scaled / state.total_shares;
        // Exactly what was not credited to `acc_fee_per_share`, kept in scaled
        // units so nothing is ever double-counted or lost.
        let remainder = sub(pool_scaled, mul(delta, state.total_shares)?)?;
        (delta, remainder)
    };
    let carry = remainder_scaled / ACC_PRECISION;
    let distributed = sub(pool, carry)?;

    state.acc_fee_per_share = add(state.acc_fee_per_share, per_share_delta)?;
    state.carry = carry;
    state.dust_scaled = remainder_scaled % ACC_PRECISION;
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
    // `accumulated` can sit just below `fee_debt` right after a share-change
    // checkpoint (the debt is rounded up).  Nothing is owed yet in that case,
    // and the debt must not be lowered — doing so would pay the rounding
    // difference out a second time.
    if accumulated <= position.fee_debt {
        return Ok(0);
    }
    let newly = sub(accumulated, position.fee_debt)?;
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
    position.fee_debt = debt_checkpoint(position.shares, state.acc_fee_per_share)?;
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
    position.fee_debt = debt_checkpoint(position.shares, state.acc_fee_per_share)?;
    events::emit_shares_changed(env, &position.staker, position.shares, state.total_shares);
    Ok(())
}

/// Slash `amount` shares from `position` without touching rewards it has
/// already earned (Issue #1205).  See the module docs for the ordering rules.
///
/// `amount` is clamped to `position.shares`, mirroring the vault's stake slash
/// (a slash can never exceed what is staked), so `amount >= shares` is a full
/// slash.  `position.realized` is never reduced.
///
/// # Errors
/// * [`FeeAccrualError::InvalidAmount`] — `amount <= 0`.
/// * [`FeeAccrualError::InsufficientShares`] — the position holds no shares.
/// * [`FeeAccrualError::Overflow`] — an intermediate calculation overflowed.
///
/// On error, neither `state` nor `position` is modified.
pub fn apply_slash(
    env: &Env,
    state: &mut FeeAccrualState,
    position: &mut StakerFeePosition,
    amount: i128,
) -> Result<SlashSettlement, FeeAccrualError> {
    if amount <= 0 {
        return Err(FeeAccrualError::InvalidAmount);
    }
    if position.shares <= 0 {
        return Err(FeeAccrualError::InsufficientShares);
    }
    let slashed = if amount > position.shares {
        position.shares
    } else {
        amount
    };

    // Compute on copies so a failure cannot leave a half-applied slash.
    let mut next_state = state.clone();
    let mut next_position = position.clone();
    let shares_before = next_position.shares;

    let settled = settle(&next_state, &mut next_position)?;
    next_position.shares = sub(next_position.shares, slashed)?;
    next_state.total_shares = sub(next_state.total_shares, slashed)?;
    next_position.fee_debt = debt_checkpoint(next_position.shares, next_state.acc_fee_per_share)?;

    *state = next_state;
    *position = next_position;

    let outcome = SlashSettlement {
        settled,
        slashed_shares: slashed,
        shares_before,
        shares_after: position.shares,
        realized: position.realized,
        total_shares: state.total_shares,
    };
    events::emit_slash_settled(env, &position.staker, &outcome);
    Ok(outcome)
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
    use super::{AccrualUpdate, Env, SlashSettlement, Symbol};
    use shared::event_topics as topics;
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

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct EvtFeeSlashSettled {
        pub schema_version: u32,
        pub staker: Address,
        pub settled: i128,
        pub slashed_shares: i128,
        pub shares_before: i128,
        pub shares_after: i128,
        pub realized: i128,
        pub total_shares: i128,
    }

    pub(super) fn emit_slash_settled(env: &Env, staker: &Address, s: &SlashSettlement) {
        env.events().publish(
            (contract_topic(env), topics::TOPIC_FEE_SLASH_SETTLED()),
            EvtFeeSlashSettled {
                schema_version: 1,
                staker: staker.clone(),
                settled: s.settled,
                slashed_shares: s.slashed_shares,
                shares_before: s.shares_before,
                shares_after: s.shares_after,
                realized: s.realized,
                total_shares: s.total_shares,
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

        // 10 over 3 shares — not evenly divisible.  3.333… per share is
        // credited; the sub-unit remainder is kept exactly in `dust_scaled`
        // (it used to also be counted as a whole unit of `carry`, crediting
        // the same value twice and letting claims exceed deposits).
        let u = accrue_deposit(&env, &mut state, 10, 1, 1).unwrap();
        assert_eq!(u.distributed, 10);
        assert_eq!(u.carry, 0);
        assert_eq!(state.dust_scaled, 1);
        assert_eq!(pending(&state, &a).unwrap(), 3);
        assert!(pending(&state, &a).unwrap() * 3 + state.carry <= state.total_deposited);

        // Next deposit folds the dust back in: 5.000…001 over 3 shares brings
        // every position to exactly 5.
        let u2 = accrue_deposit(&env, &mut state, 5, 2, 2).unwrap();
        assert_eq!(u2.distributed, 5);
        assert_eq!(u2.carry, 0);
        assert_eq!(state.dust_scaled, 0);

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

    #[test]
    fn repeated_uneven_deposits_never_over_allocate() {
        // Regression: 10 over 3 shares, repeated, used to leave a single
        // staker able to claim 42 (+1 carry) out of 40 deposited.
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 3).unwrap();
        for i in 0..4u32 {
            accrue_deposit(&env, &mut state, 10, i + 1, i as u64 + 1).unwrap();
            assert_solvent(&state, &[&a], i as i128 + 1);
            assert_eq!(state.total_deposited, state.total_distributed + state.carry);
        }
        assert_eq!(pending(&state, &a).unwrap(), 39);
    }

    #[test]
    fn share_changes_never_over_allocate() {
        // Regression: re-checkpointing `fee_debt` with floor rounding after a
        // share change paid the rounding difference out again later.
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        let mut c = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 333).unwrap();
        add_shares(&env, &mut state, &mut b, 211).unwrap();
        add_shares(&env, &mut state, &mut c, 97).unwrap();
        for (i, extra) in [7i128, 50, 13, 200].iter().enumerate() {
            accrue_deposit(&env, &mut state, 1_001, i as u32 + 1, i as u64 + 1).unwrap();
            add_shares(&env, &mut state, &mut a, *extra).unwrap();
            settle(&state, &mut a).unwrap(); // settle right after a checkpoint
            assert_solvent(&state, &[&a, &b, &c], i as i128 + 1);
        }
    }

    // ── Issue #1205: slashing preserves accrued rewards ──────────────────────

    fn assert_share_invariant(state: &FeeAccrualState, positions: &[&StakerFeePosition]) {
        let sum: i128 = positions.iter().map(|p| p.shares).sum();
        assert_eq!(sum, state.total_shares, "share invariant violated");
    }

    /// For uneven amounts each position floors independently, so up to one
    /// unit per position per deposit can be stranded (true without slashing
    /// too).  What must never happen is over-allocation.
    fn assert_solvent(state: &FeeAccrualState, positions: &[&StakerFeePosition], deposits: i128) {
        let claimable: i128 = positions.iter().map(|p| pending(state, p).unwrap()).sum();
        let accounted = claimable + state.carry;
        assert!(accounted <= state.total_deposited, "over-allocated rewards");
        // At most ~1 unit per position per checkpoint/deposit, plus the
        // sub-unit dust still held in `dust_scaled`.
        assert!(
            state.total_deposited - accounted <= 2 * positions.len() as i128 * (deposits + 1),
            "rounding dust exceeded bound"
        );
    }

    #[test]
    fn partial_slash_preserves_pending_rewards() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 100).unwrap();
        add_shares(&env, &mut state, &mut b, 100).unwrap();
        accrue_deposit(&env, &mut state, 1_000, 1, 1).unwrap();
        assert_eq!(pending(&state, &a).unwrap(), 500);

        // Slash 40% of A's principal.
        let out = apply_slash(&env, &mut state, &mut a, 40).unwrap();
        assert_eq!(out.settled, 500);
        assert_eq!(out.slashed_shares, 40);
        assert_eq!(out.shares_before, 100);
        assert_eq!(out.shares_after, 60);
        assert_eq!(out.realized, 500);
        assert_eq!(out.total_shares, 160);

        // Earned-before-slash rewards are intact.
        assert_eq!(pending(&state, &a).unwrap(), 500);
        assert_eq!(pending(&state, &b).unwrap(), 500);
        assert_share_invariant(&state, &[&a, &b]);
        assert_conservation(&state, &[&a, &b]);
    }

    #[test]
    fn accrual_after_partial_slash_uses_remaining_shares_only() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 100).unwrap();
        add_shares(&env, &mut state, &mut b, 100).unwrap();
        accrue_deposit(&env, &mut state, 1_000, 1, 1).unwrap();

        apply_slash(&env, &mut state, &mut a, 60).unwrap(); // A: 40, B: 100

        // 1_400 over 140 shares => 10 per share: A +400, B +1_000.
        accrue_deposit(&env, &mut state, 1_400, 2, 2).unwrap();
        assert_eq!(pending(&state, &a).unwrap(), 500 + 400);
        assert_eq!(pending(&state, &b).unwrap(), 500 + 1_000);
        assert_share_invariant(&state, &[&a, &b]);
        assert_conservation(&state, &[&a, &b]);
    }

    #[test]
    fn full_slash_keeps_realized_and_stops_future_accrual() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 30).unwrap();
        add_shares(&env, &mut state, &mut b, 70).unwrap();
        accrue_deposit(&env, &mut state, 1_000, 1, 1).unwrap();

        let out = apply_slash(&env, &mut state, &mut a, 30).unwrap();
        assert_eq!(out.shares_after, 0);
        assert_eq!(out.realized, 300);
        assert_eq!(state.total_shares, 70);

        // All later accrual goes to B; A's pending is frozen at what it earned.
        accrue_deposit(&env, &mut state, 700, 2, 2).unwrap();
        assert_eq!(pending(&state, &a).unwrap(), 300);
        assert_eq!(pending(&state, &b).unwrap(), 700 + 700);
        assert_share_invariant(&state, &[&a, &b]);
        assert_conservation(&state, &[&a, &b]);

        // The fully slashed staker can still claim what was earned.
        assert_eq!(claim(&env, &state, &mut a).unwrap(), 300);
    }

    #[test]
    fn oversized_slash_is_clamped_to_full_slash() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 10).unwrap();
        accrue_deposit(&env, &mut state, 100, 1, 1).unwrap();

        let out = apply_slash(&env, &mut state, &mut a, 1_000).unwrap();
        assert_eq!(out.slashed_shares, 10);
        assert_eq!(a.shares, 0);
        assert_eq!(state.total_shares, 0);
        assert_eq!(a.realized, 100);
    }

    #[test]
    fn slash_with_unclaimed_realized_and_unsettled_rewards() {
        // A has both already-realized fees (from an earlier settle) and
        // unsettled accrual when the slash lands; both must survive.
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 50).unwrap();
        add_shares(&env, &mut state, &mut b, 50).unwrap();
        accrue_deposit(&env, &mut state, 200, 1, 1).unwrap();
        settle(&state, &mut a).unwrap(); // realized = 100
        accrue_deposit(&env, &mut state, 400, 2, 2).unwrap(); // +200 unsettled

        let out = apply_slash(&env, &mut state, &mut a, 25).unwrap();
        assert_eq!(out.settled, 200);
        assert_eq!(out.realized, 300);
        assert_eq!(pending(&state, &a).unwrap(), 300);
        assert_conservation(&state, &[&a, &b]);
    }

    #[test]
    fn slash_rejections_leave_state_unchanged() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut empty = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 10).unwrap();
        accrue_deposit(&env, &mut state, 100, 1, 1).unwrap();

        let (state_before, a_before) = (state.clone(), a.clone());
        assert_eq!(
            apply_slash(&env, &mut state, &mut a, 0),
            Err(FeeAccrualError::InvalidAmount)
        );
        assert_eq!(
            apply_slash(&env, &mut state, &mut a, -1),
            Err(FeeAccrualError::InvalidAmount)
        );
        assert_eq!(
            apply_slash(&env, &mut state, &mut empty, 5),
            Err(FeeAccrualError::InsufficientShares)
        );
        assert_eq!(state, state_before);
        assert_eq!(a, a_before);
    }

    #[test]
    fn repeated_partial_slashes_keep_invariants() {
        let (env, mut state) = setup();
        let mut a = StakerFeePosition::new(Address::generate(&env));
        let mut b = StakerFeePosition::new(Address::generate(&env));
        let mut c = StakerFeePosition::new(Address::generate(&env));
        add_shares(&env, &mut state, &mut a, 333).unwrap();
        add_shares(&env, &mut state, &mut b, 211).unwrap();
        add_shares(&env, &mut state, &mut c, 97).unwrap();

        for (i, slash) in [7i128, 50, 13, 200].into_iter().enumerate() {
            let ledger = i as u32 + 1;
            accrue_deposit(&env, &mut state, 1_001, ledger, ledger as u64).unwrap();
            let before = pending(&state, &a).unwrap();
            apply_slash(&env, &mut state, &mut a, slash).unwrap();
            assert_eq!(pending(&state, &a).unwrap(), before, "slash erased rewards");
            assert_share_invariant(&state, &[&a, &b, &c]);
            assert_solvent(&state, &[&a, &b, &c], ledger as i128);
        }
        assert_eq!(a.shares, 333 - 7 - 50 - 13 - 200);
    }
}
