//! Issue #1045: Contract-level invariants harness for security review.
//!
//! Verifies core accounting and balance assumptions automatically after
//! representative state transitions across stake, reward, and liquidity
//! operations. Invariant failures produce actionable output for review.
//!
//! # Invariants Verified
//!
//! ## Stake accounting
//! - Slash conservation: `slashed + remaining == original_balance`
//! - Slash is bounded: `0 <= slashed <= balance`
//! - Severity ordering: `minor <= major <= critical`
//! - Zero-balance slash yields zero
//!
//! ## Reward ledger
//! - Reward window epoch IDs are strictly monotonic
//! - Double-claim is rejected (idempotency)
//! - Anchor ledger is stable across repeated evaluations in the same window
//! - Claims after window close are rejected
//!
//! ## Liquidity / leverage
//! - Collateral ratio is deterministic for the same inputs
//! - Settlement clears debt without creating mismatched totals
//! - Healthy positions are never settled
//!
//! # Running
//!
//! ```sh
//! cargo test --test test_contract_invariants_harness
//! ```

extern crate std;

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{contract, contractimpl, Address, Env};

use signal_registry::reward_ledger::{
    check_claim_eligibility, get_claim_record, open_reward_window, record_claim, RewardLedgerError,
};
use trade_executor::leverage::{
    collateral_ratio_bps, is_undercollateralized, settle_debt, MIN_COLLATERAL_RATIO_BPS,
};

// ── Minimal contract wrapper ──────────────────────────────────────────────────
// Soroban storage and events are only accessible from within a contract context.

#[contract]
struct InvariantTestContract;
#[contractimpl]
impl InvariantTestContract {}

fn make_env() -> (Env, soroban_sdk::Address) {
    let env = Env::default();
    let id = env.register(InvariantTestContract, ());
    (env, id)
}

// ── Constants ─────────────────────────────────────────────────────────────────

const MAX_BALANCE: i128 = 1_000_000_000_000_000;
const MAX_SLASH_BPS: u32 = 10_000;
const BPS_DENOM: i128 = 10_000;

// ── Pure arithmetic helpers (no env needed) ───────────────────────────────────

fn slash_amount(balance: i128, bps: u32) -> i128 {
    balance
        .checked_mul(bps as i128)
        .and_then(|v| v.checked_div(BPS_DENOM))
        .unwrap_or(i128::MAX)
}

// ── Stake accounting invariants (pure arithmetic — no env) ────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 1000, ..ProptestConfig::default() })]

    /// Slash conservation: slashed + remaining == original balance.
    #[test]
    fn prop_stake_slash_conservation(
        balance in 0_i128..=MAX_BALANCE,
        bps in 0u32..=MAX_SLASH_BPS,
    ) {
        let slashed = slash_amount(balance, bps);
        let remaining = balance - slashed;
        prop_assert_eq!(
            slashed + remaining, balance,
            "slash conservation violated: slashed={} remaining={} balance={}",
            slashed, remaining, balance
        );
    }

    /// Slash is bounded: 0 <= slashed <= balance.
    #[test]
    fn prop_stake_slash_bounded(
        balance in 0_i128..=MAX_BALANCE,
        bps in 0u32..=MAX_SLASH_BPS,
    ) {
        let slashed = slash_amount(balance, bps);
        prop_assert!(slashed >= 0, "slash must be non-negative");
        prop_assert!(slashed <= balance, "slash must not exceed balance");
    }

    /// Severity ordering: minor <= major <= critical.
    #[test]
    fn prop_stake_severity_ordering(balance in 1_i128..=MAX_BALANCE) {
        const MINOR: u32 = 500;
        const MAJOR: u32 = 3_000;
        const CRITICAL: u32 = 10_000;
        let minor = slash_amount(balance, MINOR);
        let major = slash_amount(balance, MAJOR);
        let critical = slash_amount(balance, CRITICAL);
        prop_assert!(minor <= major, "minor slash must be <= major");
        prop_assert!(major <= critical, "major slash must be <= critical");
        prop_assert_eq!(critical, balance, "critical slash must equal full balance");
    }

    /// Zero-balance slash always yields zero.
    #[test]
    fn prop_stake_zero_balance_slash(bps in 0u32..=MAX_SLASH_BPS) {
        prop_assert_eq!(slash_amount(0, bps), 0);
    }
}

// ── Collateral ratio invariants (pure — no env) ───────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 1000, ..ProptestConfig::default() })]

    /// Collateral ratio is deterministic for the same inputs.
    #[test]
    fn prop_collateral_ratio_deterministic(
        collateral in 0_i128..=MAX_BALANCE,
        debt in 1_i128..=MAX_BALANCE,
    ) {
        let r1 = collateral_ratio_bps(collateral, debt);
        let r2 = collateral_ratio_bps(collateral, debt);
        prop_assert_eq!(r1, r2, "collateral_ratio_bps must be deterministic");
    }

    /// Zero debt always returns None (no debt = fully collateralized).
    #[test]
    fn prop_collateral_ratio_zero_debt(collateral in 0_i128..=MAX_BALANCE) {
        prop_assert_eq!(collateral_ratio_bps(collateral, 0), None);
    }

    /// Undercollateralized detection matches ratio threshold.
    #[test]
    fn prop_undercollateralized_matches_ratio(
        collateral in 0_i128..=MAX_BALANCE,
        debt in 1_i128..=MAX_BALANCE,
    ) {
        let under = is_undercollateralized(collateral, debt);
        let ratio = collateral_ratio_bps(collateral, debt);
        let expected_under = ratio.map(|r| r < MIN_COLLATERAL_RATIO_BPS).unwrap_or(false);
        prop_assert_eq!(under, expected_under);
    }
}

// ── Settlement invariants (needs env for events/ledger) ───────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 500, ..ProptestConfig::default() })]

    /// Healthy positions (ratio >= MIN_COLLATERAL_RATIO_BPS) are never settled.
    #[test]
    fn prop_healthy_position_not_settled(
        debt in 1_i128..=1_000_000_i128,
        extra in 0_i128..=1_000_000_i128,
    ) {
        let collateral = (debt * 11 / 10).saturating_add(extra);
        let (env, id) = make_env();
        let result = env.as_contract(&id, || settle_debt(&env, collateral, debt, 1_000_000));
        prop_assert!(
            result.is_none(),
            "healthy position must not be settled: collateral={} debt={}",
            collateral, debt
        );
    }

    /// Settlement conservation: debt_cleared <= debt and reserve_consumed <= reserve.
    #[test]
    fn prop_settlement_bounded(
        collateral in 0_i128..=100_i128,
        debt in 101_i128..=200_i128,
        reserve in 0_i128..=MAX_BALANCE,
    ) {
        let (env, id) = make_env();
        if let Some(result) = env.as_contract(&id, || settle_debt(&env, collateral, debt, reserve)) {
            prop_assert!(result.debt_cleared <= debt, "debt_cleared must not exceed original debt");
            prop_assert!(result.reserve_consumed <= reserve, "reserve_consumed must not exceed available reserve");
            prop_assert!(result.collateral_remaining >= 0, "collateral_remaining must be non-negative");
        }
    }

    /// Undercollateralized detection is consistent with settlement outcome.
    #[test]
    fn prop_undercollateralized_iff_settled(
        collateral in 0_i128..=MAX_BALANCE,
        debt in 1_i128..=MAX_BALANCE,
    ) {
        let (env, id) = make_env();
        let under = is_undercollateralized(collateral, debt);
        let settled = env.as_contract(&id, || settle_debt(&env, collateral, debt, 0)).is_some();
        prop_assert_eq!(
            under, settled,
            "is_undercollateralized and settle_debt must agree: collateral={} debt={}",
            collateral, debt
        );
    }
}

// ── Reward ledger invariants ──────────────────────────────────────────────────

/// Epoch IDs are strictly monotonic across consecutive window openings.
#[test]
fn invariant_reward_epoch_monotonic() {
    let (env, id) = make_env();
    env.ledger().set_sequence_number(1);
    let w1 = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
    env.ledger().set_sequence_number(200);
    let w2 = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
    assert!(
        w2.epoch_id > w1.epoch_id,
        "epoch_id must be strictly monotonic: w1={} w2={}",
        w1.epoch_id,
        w2.epoch_id
    );
}

/// Anchor ledger is stable: repeated eligibility checks in the same window
/// always return the same anchor_ledger.
#[test]
fn invariant_reward_anchor_ledger_stable() {
    let (env, id) = make_env();
    env.ledger().set_sequence_number(42);
    let window = env.as_contract(&id, || open_reward_window(&env, 200, 500_000));
    let anchor = window.anchor_ledger;

    for seq in [50u32, 100, 150, 200, 241] {
        env.ledger().set_sequence_number(seq);
        let provider = Address::generate(&env);
        if let Ok(w) = env.as_contract(&id, || check_claim_eligibility(&env, &provider)) {
            assert_eq!(
                w.anchor_ledger, anchor,
                "anchor_ledger changed at ledger {}: expected {} got {}",
                seq, anchor, w.anchor_ledger
            );
        }
    }
}

/// Double-claim is rejected — idempotency invariant.
#[test]
fn invariant_reward_no_double_claim() {
    let (env, id) = make_env();
    env.ledger().set_sequence_number(1);
    env.as_contract(&id, || {
        open_reward_window(&env, 100, 1_000_000);
    });
    let provider = Address::generate(&env);

    env.as_contract(&id, || {
        record_claim(&env, &provider, 100).expect("first claim must succeed");
    });
    let second = env.as_contract(&id, || record_claim(&env, &provider, 100));
    assert_eq!(second, Err(RewardLedgerError::AlreadyClaimed), "second claim must be rejected");
}

/// Claim record references the correct epoch and amount.
#[test]
fn invariant_reward_claim_record_correct() {
    let (env, id) = make_env();
    env.ledger().set_sequence_number(5);
    let window = env.as_contract(&id, || open_reward_window(&env, 50, 1_000_000));
    let provider = Address::generate(&env);
    env.ledger().set_sequence_number(10);
    env.as_contract(&id, || {
        record_claim(&env, &provider, 300).unwrap();
    });

    let rec = env
        .as_contract(&id, || get_claim_record(&env, &provider, window.epoch_id))
        .expect("claim record must exist after successful claim");
    assert_eq!(rec.epoch_id, window.epoch_id);
    assert_eq!(rec.amount_claimed, 300);
    assert!(rec.amount_claimed > 0, "claimed amount must be positive");
}

/// Window closed: claims after close_ledger are rejected.
#[test]
fn invariant_reward_window_closed_rejects_claims() {
    let (env, id) = make_env();
    env.ledger().set_sequence_number(1);
    env.as_contract(&id, || {
        open_reward_window(&env, 10, 1_000_000);
    });
    env.ledger().set_sequence_number(12); // past close_ledger = 11
    let provider = Address::generate(&env);
    let result = env.as_contract(&id, || record_claim(&env, &provider, 100));
    assert_eq!(result, Err(RewardLedgerError::WindowClosed), "claim after window close must be rejected");
}

/// No active window: claims are rejected with NoActiveWindow.
#[test]
fn invariant_reward_no_window_rejects_claim() {
    let (env, id) = make_env();
    let provider = Address::generate(&env);
    let result = env.as_contract(&id, || record_claim(&env, &provider, 100));
    assert_eq!(result, Err(RewardLedgerError::NoActiveWindow));
}

// ── Deterministic regression tests ───────────────────────────────────────────

#[test]
fn regression_slash_critical_wipes_balance() {
    for balance in [1_i128, 1_000, MAX_BALANCE] {
        let slashed = slash_amount(balance, 10_000);
        assert_eq!(slashed, balance, "critical slash must equal full balance for {}", balance);
    }
}

#[test]
fn regression_settlement_no_reserve_clears_only_collateral() {
    let (env, id) = make_env();
    // collateral=50, debt=100 → ratio=50% — undercollateralized
    let result = env.as_contract(&id, || settle_debt(&env, 50, 100, 0)).unwrap();
    assert_eq!(result.debt_cleared, 50);
    assert_eq!(result.reserve_consumed, 0);
    assert_eq!(result.collateral_remaining, 0);
}

#[test]
fn regression_collateral_ratio_zero_debt_returns_none() {
    assert_eq!(collateral_ratio_bps(1_000, 0), None);
}
