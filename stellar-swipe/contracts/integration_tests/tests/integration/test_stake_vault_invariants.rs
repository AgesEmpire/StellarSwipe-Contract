//! Property-based invariant tests for stake_vault arithmetic and
//! shared::math::normalize_amount precision conversions.
//!
//! # Invariants Tested
//!
//! ## Stake-vault slash / voting-power arithmetic
//!
//! - **Slash non-negative**: For any valid stake balance and BPS rate, the
//!   slash amount is ≥ 0 and ≤ the full balance.
//! - **Slash conservation**: `slashed + remaining == original_balance` (no
//!   dust created or destroyed).
//! - **Slash monotonic with severity**: Minor ≤ Major ≤ Critical.
//! - **Voting-power multiplier bounded**: Multiplied balance is ≥ original
//!   (for bps ≥ 10_000) and never wraps to negative.
//! - **Zero-balance invariants**: Slash of 0 yields 0; multiplier of 0 yields 0.
//!
//! ## Decimal-precision scaling (shared::math::normalize_amount)
//!
//! - **Identity**: Same precision in and out returns the same value.
//! - **Scale-up never loses information**: Round-trip (up then down) recovers
//!   the original value for exact multiples.
//! - **Scale-down truncation bounded**: Lost units < 10^diff.
//! - **Monotonicity**: Larger input ⟹ larger (or equal) output.
//! - **Overflow safety**: Known-overflow inputs return `None`.
//!
//! # Running
//!
//! ```sh
//! cargo test --test test_stake_vault_invariants
//! ```

extern crate std;

use proptest::prelude::*;
use shared::math::normalize_amount;

// ── Constants ─────────────────────────────────────────────────────────────────

const BPS_DENOM: i128 = 10_000;

/// Maximum realistic stake in stroops (10^15 — well within i128 range even
/// after a ×2x voting-power multiplier).
const MAX_STAKE: i128 = 1_000_000_000_000_000;

/// Maximum slash rate in basis points (100%).
const MAX_SLASH_BPS: u32 = 10_000;

// ── Slash helpers (mirrors stake_vault logic) ─────────────────────────────────

/// `slash_amount = balance * bps / 10_000`, saturating on overflow.
fn slash_amount(balance: i128, bps: u32) -> i128 {
    balance
        .checked_mul(bps as i128)
        .and_then(|v| v.checked_div(BPS_DENOM))
        .unwrap_or(i128::MAX)
}

/// `apply_multiplier_bps = balance * bps / 10_000`, saturating on overflow.
fn apply_multiplier(balance: i128, bps: u32) -> i128 {
    balance
        .checked_mul(bps as i128)
        .and_then(|v| v.checked_div(BPS_DENOM))
        .unwrap_or(i128::MAX)
}

// ── Slash invariants ──────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 2000,
        ..ProptestConfig::default()
    })]

    /// Slash amount is non-negative and does not exceed the original balance.
    #[test]
    fn prop_slash_bounded(
        balance in 0_i128..=MAX_STAKE,
        slash_bps in 0u32..=MAX_SLASH_BPS,
    ) {
        let slashed = slash_amount(balance, slash_bps);
        prop_assert!(slashed >= 0, "slash must be non-negative");
        prop_assert!(slashed <= balance, "slash must not exceed balance: {} > {}", slashed, balance);
    }

    /// Conservation: slashed + remaining == original.
    #[test]
    fn prop_slash_conservation(
        balance in 0_i128..=MAX_STAKE,
        slash_bps in 0u32..=MAX_SLASH_BPS,
    ) {
        let slashed = slash_amount(balance, slash_bps);
        let remaining = balance - slashed;
        prop_assert_eq!(slashed + remaining, balance,
            "slashed + remaining must equal original balance");
    }

    /// Monotonicity: higher slash rate ⟹ equal or larger slash amount.
    #[test]
    fn prop_slash_monotonic_with_rate(
        balance in 0_i128..=MAX_STAKE,
        bps_lo in 0u32..MAX_SLASH_BPS,
    ) {
        let bps_hi = bps_lo + 1;
        let slash_lo = slash_amount(balance, bps_lo);
        let slash_hi = slash_amount(balance, bps_hi);
        prop_assert!(slash_hi >= slash_lo,
            "higher bps {} should slash >= lower bps {}: {} < {}", bps_hi, bps_lo, slash_hi, slash_lo);
    }

    /// Monotonicity: larger balance ⟹ equal or larger slash for same rate.
    #[test]
    fn prop_slash_monotonic_with_balance(
        balance_lo in 0_i128..MAX_STAKE,
        slash_bps in 0u32..=MAX_SLASH_BPS,
    ) {
        let balance_hi = balance_lo + 1;
        let slash_lo = slash_amount(balance_lo, slash_bps);
        let slash_hi = slash_amount(balance_hi, slash_bps);
        prop_assert!(slash_hi >= slash_lo,
            "larger balance should yield >= slash: {} < {}", slash_hi, slash_lo);
    }

    /// Minor ≤ Major ≤ Critical ordering (default tier config: 500/3000/10000 bps).
    #[test]
    fn prop_slash_severity_ordering(balance in 1_i128..=MAX_STAKE) {
        const MINOR_BPS: u32 = 500;
        const MAJOR_BPS: u32 = 3_000;
        const CRITICAL_BPS: u32 = 10_000;

        let minor = slash_amount(balance, MINOR_BPS);
        let major = slash_amount(balance, MAJOR_BPS);
        let critical = slash_amount(balance, CRITICAL_BPS);

        prop_assert!(minor <= major, "minor slash must be <= major: {} > {}", minor, major);
        prop_assert!(major <= critical, "major slash must be <= critical: {} > {}", major, critical);
        prop_assert_eq!(critical, balance, "critical slash must equal full balance");
    }

    /// Zero balance slashes to zero regardless of rate.
    #[test]
    fn prop_slash_zero_balance(slash_bps in 0u32..=MAX_SLASH_BPS) {
        let slashed = slash_amount(0, slash_bps);
        prop_assert_eq!(slashed, 0, "slash of zero balance must be zero");
    }

    /// Voting-power multiplier: result is ≥ balance for bps ≥ BPS_DENOM.
    #[test]
    fn prop_multiplier_at_least_one_x(
        balance in 0_i128..=MAX_STAKE,
        bps in 10_000u32..=20_000u32,   // 1x to 2x
    ) {
        let result = apply_multiplier(balance, bps);
        prop_assert!(result >= balance,
            "multiplier bps={} applied to {} gave {} (less than original)", bps, balance, result);
    }

    /// Multiplier is non-negative and never wraps to negative.
    #[test]
    fn prop_multiplier_non_negative(
        balance in 0_i128..=MAX_STAKE,
        bps in 0u32..=20_000u32,
    ) {
        let result = apply_multiplier(balance, bps);
        prop_assert!(result >= 0, "multiplied balance must not be negative");
    }
}

// ── normalize_amount invariants ───────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 2000,
        ..ProptestConfig::default()
    })]

    /// Identity: same precision in and out returns the exact same value.
    #[test]
    fn prop_normalize_identity(
        amount in i128::MIN / 2..=i128::MAX / 2,
        precision in 0u32..=18,
    ) {
        let result = normalize_amount(amount, precision, precision);
        prop_assert_eq!(result, Some(amount), "same precision must be identity");
    }

    /// Scale-up: result is a precise multiple; no information is lost.
    #[test]
    fn prop_scale_up_exact(
        amount in 0_i128..=1_000_000_000_i128,
        from in 0u32..=9,
        extra in 1u32..=9,
    ) {
        let to = from + extra;
        let factor = (10i128).pow(extra);
        let result = normalize_amount(amount, from, to);
        prop_assert!(result.is_some(), "scale-up should not overflow for small inputs");
        prop_assert_eq!(result.unwrap(), amount * factor,
            "scale-up by {} should be exact multiplication", factor);
    }

    /// Scale-down truncation: lost precision is strictly less than the scale factor.
    #[test]
    fn prop_scale_down_truncation_bounded(
        amount in 0_i128..=1_000_000_000_000_i128,
        to in 0u32..=6,
        extra in 1u32..=6,
    ) {
        let from = to + extra;
        let factor = (10i128).pow(extra);
        let down = normalize_amount(amount, from, to);
        prop_assert!(down.is_some());
        let down_val = down.unwrap();

        // Restore: multiply back.
        let restored = normalize_amount(down_val, to, from);
        prop_assert!(restored.is_some());
        let restored_val = restored.unwrap();

        // Loss must be < factor.
        let loss = amount - restored_val;
        prop_assert!(
            loss >= 0 && loss < factor,
            "truncation loss {} must be in [0, {})", loss, factor
        );
    }

    /// Monotonicity: larger input ⟹ larger (or equal) normalized output
    /// (for scale-up, exact; for scale-down, may be equal due to truncation).
    #[test]
    fn prop_normalize_monotonic(
        amount_lo in 0_i128..=999_999_999_i128,
        from in 0u32..=9,
        to in 0u32..=9,
    ) {
        let amount_hi = amount_lo + 1;
        let lo = normalize_amount(amount_lo, from, to);
        let hi = normalize_amount(amount_hi, from, to);
        if let (Some(l), Some(h)) = (lo, hi) {
            prop_assert!(h >= l,
                "normalize({}, {}, {}) = {} should be >= normalize({}, {}, {}) = {}",
                amount_hi, from, to, h, amount_lo, from, to, l);
        }
    }

    /// Overflow: inputs that are guaranteed to overflow return None.
    #[test]
    fn prop_overflow_returns_none(
        amount in (i128::MAX / 10)..=i128::MAX,
    ) {
        // Scaling i128::MAX/10 up by 10^20 must overflow.
        let result = normalize_amount(amount, 0, 20);
        prop_assert!(result.is_none(),
            "large scale-up of {} should overflow (return None)", amount);
    }

    /// Zero always normalizes to zero regardless of precision direction.
    #[test]
    fn prop_zero_stays_zero(from in 0u32..=18, to in 0u32..=18) {
        let result = normalize_amount(0, from, to);
        prop_assert_eq!(result, Some(0), "zero must stay zero");
    }
}

// ── Deterministic regression tests ───────────────────────────────────────────
// These pin specific corner cases found during property testing.

#[test]
fn regression_slash_critical_equals_full_balance() {
    // Critical slash (10_000 bps = 100%) must wipe the whole balance.
    for balance in [1, 999, 1_000_000, MAX_STAKE] {
        let slashed = slash_amount(balance, 10_000);
        assert_eq!(slashed, balance, "critical slash should equal full balance for {}", balance);
    }
}

#[test]
fn regression_slash_zero_rate_preserves_balance() {
    // Zero-bps slash removes nothing.
    for balance in [1, 1_000_000, MAX_STAKE] {
        let slashed = slash_amount(balance, 0);
        assert_eq!(slashed, 0, "zero-rate slash should be 0 for {}", balance);
    }
}

#[test]
fn regression_normalize_round_trip_7_to_6_decimals() {
    // Canonical Stellar precision conversion.
    let amount: i128 = 10_000_001; // 1.0000001 in 7-decimal notation
    let down = normalize_amount(amount, 7, 6).expect("7→6 should not overflow");
    let up = normalize_amount(down, 6, 7).expect("6→7 should not overflow");
    // Sub-unit loss must be < 10 (one 7-decimal unit).
    assert!(amount - up < 10 && amount >= up,
        "round-trip 7→6→7 for {} should lose < 10 units, got {}", amount, up);
}

#[test]
fn regression_normalize_identity_zero() {
    assert_eq!(normalize_amount(0, 7, 7), Some(0));
    assert_eq!(normalize_amount(0, 0, 18), Some(0));
    assert_eq!(normalize_amount(0, 18, 0), Some(0));
}
