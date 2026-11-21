//! Real-time TVL tracking across all vaults (Issue #673).
//!
//! Aggregates balances from stake_vault, liquidity_pool, and escrow/holding
//! into a single TVL figure updated incrementally on each deposit/withdrawal.
//!
//! ## Deterministic multi-asset rounding (Issue #1447)
//!
//! Each source may hold an asset with its own decimal precision. Before a
//! raw asset amount is folded into the aggregate TVL total it is converted
//! to [`BASE_DECIMALS`] (7, matching the Stellar/Soroban standard) via
//! [`convert_to_base_units`]. The conversion:
//!
//! - **Truncates toward zero** when scaling down (more asset decimals than
//!   base decimals) — the fractional base-unit remainder is dropped, never
//!   rounded up. This matches Rust's native integer division, so the
//!   behavior is the same for every caller and every build, not an
//!   incidental platform detail.
//! - Uses **checked** multiplication/division throughout; any overflow
//!   returns [`TvlError::Overflow`] instead of silently wrapping or
//!   saturating, because a wrapped/saturated conversion would corrupt the
//!   aggregate total rather than merely rounding it.
//! - Is applied **per asset, independently**, before the result is added to
//!   the running per-source total. Summation is commutative and each
//!   asset's contribution is rounded exactly once regardless of how many
//!   other deposits/withdrawals happen before or after it, so the final
//!   aggregate TVL is the same regardless of the order operations arrive
//!   in.

use soroban_sdk::{contracterror, contracttype, Env, Symbol};

/// Base precision (decimal places) that all TVL figures are expressed in,
/// matching the Stellar/Soroban standard asset precision.
pub const BASE_DECIMALS: u32 = 7;

/// Errors from converting or aggregating multi-asset TVL contributions.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum TvlError {
    /// The conversion or aggregation arithmetic overflowed.
    Overflow = 1,
}

/// Converts `raw_amount`, denominated in an asset with `asset_decimals`
/// decimal places, into [`BASE_DECIMALS`]-precision base units.
///
/// Rounding direction is documented and fixed: scaling down **truncates
/// toward zero** (the fractional remainder below one base unit is dropped),
/// scaling up is exact (multiplication introduces no fractional loss). See
/// the module docs for why this direction was chosen and why it is stable
/// across input ordering.
pub fn convert_to_base_units(raw_amount: i128, asset_decimals: u32) -> Result<i128, TvlError> {
    if asset_decimals == BASE_DECIMALS {
        return Ok(raw_amount);
    }
    if asset_decimals > BASE_DECIMALS {
        let scale = 10i128
            .checked_pow(asset_decimals - BASE_DECIMALS)
            .ok_or(TvlError::Overflow)?;
        // Integer division truncates toward zero for i128, which is exactly
        // the documented rounding direction.
        raw_amount.checked_div(scale).ok_or(TvlError::Overflow)
    } else {
        let scale = 10i128
            .checked_pow(BASE_DECIMALS - asset_decimals)
            .ok_or(TvlError::Overflow)?;
        raw_amount.checked_mul(scale).ok_or(TvlError::Overflow)
    }
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Source categories contributing to TVL.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TvlSource {
    StakeVault,
    LiquidityPool,
    Escrow,
}

/// Schema version for `TvlSnapshot` (Issue #1445). Bump whenever a field is
/// added, removed, or reinterpreted so consumers can detect breaking changes
/// instead of silently misreading an incompatible payload.
pub const TVL_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[contracttype]
#[derive(Clone, Debug)]
pub struct TvlSnapshot {
    pub schema_version: u32,
    pub stake_vault: i128,
    pub liquidity_pool: i128,
    pub escrow: i128,
    pub total: i128,
    pub last_updated: u64,
}

#[contracttype]
#[derive(Clone)]
enum TvlKey {
    Snapshot,
}

// ── Storage ───────────────────────────────────────────────────────────────────

fn load(env: &Env) -> TvlSnapshot {
    env.storage()
        .instance()
        .get(&TvlKey::Snapshot)
        .unwrap_or(TvlSnapshot {
            schema_version: TVL_SNAPSHOT_SCHEMA_VERSION,
            stake_vault: 0,
            liquidity_pool: 0,
            escrow: 0,
            total: 0,
            last_updated: 0,
        })
}

fn save(env: &Env, snapshot: &TvlSnapshot) {
    env.storage().instance().set(&TvlKey::Snapshot, snapshot);
}

// ── Incremental updates ───────────────────────────────────────────────────────

/// Apply an incremental delta, already expressed in [`BASE_DECIMALS`] base
/// units (positive = deposit, negative = withdrawal), to the specified
/// source and update the aggregate total. Prefer [`apply_asset_delta`] when
/// the caller holds a raw, asset-native amount that still needs converting.
pub fn apply_delta(env: &Env, source: TvlSource, delta: i128) -> Result<(), TvlError> {
    let mut snap = load(env);

    match source {
        TvlSource::StakeVault => {
            snap.stake_vault = snap
                .stake_vault
                .checked_add(delta)
                .ok_or(TvlError::Overflow)?;
        }
        TvlSource::LiquidityPool => {
            snap.liquidity_pool = snap
                .liquidity_pool
                .checked_add(delta)
                .ok_or(TvlError::Overflow)?;
        }
        TvlSource::Escrow => {
            snap.escrow = snap.escrow.checked_add(delta).ok_or(TvlError::Overflow)?;
        }
    }

    // Recompute total from components to avoid accumulated drift. Checked
    // arithmetic here (issue #1447) means a corrupt/overflowing total is
    // reported as an error instead of silently saturating.
    snap.total = snap
        .stake_vault
        .checked_add(snap.liquidity_pool)
        .ok_or(TvlError::Overflow)?
        .checked_add(snap.escrow)
        .ok_or(TvlError::Overflow)?;
    snap.last_updated = env.ledger().timestamp();

    save(env, &snap);
    Ok(())
}

/// Apply an incremental delta expressed in a raw asset's own
/// `asset_decimals` precision (positive = deposit, negative = withdrawal).
/// The amount is first converted to [`BASE_DECIMALS`] base units via
/// [`convert_to_base_units`] (truncating toward zero on any precision
/// loss), then folded into the aggregate exactly like [`apply_delta`].
pub fn apply_asset_delta(
    env: &Env,
    source: TvlSource,
    raw_amount: i128,
    asset_decimals: u32,
) -> Result<(), TvlError> {
    let base_units = convert_to_base_units(raw_amount, asset_decimals)?;
    apply_delta(env, source, base_units)
}

// ── Query ─────────────────────────────────────────────────────────────────────

/// Return the current aggregated TVL snapshot.
pub fn get_protocol_tvl(env: &Env) -> TvlSnapshot {
    load(env)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    fn setup_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env
    }

    #[test]
    fn test_initial_tvl_is_zero() {
        let env = setup_env();
        let id = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id, || {
            let snap = get_protocol_tvl(&env);
            assert_eq!(snap.total, 0);
            assert_eq!(snap.stake_vault, 0);
            assert_eq!(snap.liquidity_pool, 0);
            assert_eq!(snap.escrow, 0);
        });
    }

    #[test]
    fn test_deposit_increases_tvl() {
        let env = setup_env();
        let id = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id, || {
            env.ledger().with_mut(|l| l.timestamp = 1000);
            apply_delta(&env, TvlSource::StakeVault, 500).unwrap();
            let snap = get_protocol_tvl(&env);
            assert_eq!(snap.stake_vault, 500);
            assert_eq!(snap.total, 500);
            assert_eq!(snap.last_updated, 1000);
        });
    }

    #[test]
    fn test_multi_source_aggregation() {
        let env = setup_env();
        let id = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id, || {
            apply_delta(&env, TvlSource::StakeVault, 1000).unwrap();
            apply_delta(&env, TvlSource::LiquidityPool, 2000).unwrap();
            apply_delta(&env, TvlSource::Escrow, 300).unwrap();
            let snap = get_protocol_tvl(&env);
            assert_eq!(snap.stake_vault, 1000);
            assert_eq!(snap.liquidity_pool, 2000);
            assert_eq!(snap.escrow, 300);
            assert_eq!(snap.total, 3300);
        });
    }

    #[test]
    fn test_withdrawal_decreases_tvl() {
        let env = setup_env();
        let id = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id, || {
            apply_delta(&env, TvlSource::StakeVault, 1000).unwrap();
            apply_delta(&env, TvlSource::StakeVault, -400).unwrap();
            let snap = get_protocol_tvl(&env);
            assert_eq!(snap.stake_vault, 600);
            assert_eq!(snap.total, 600);
        });
    }

    #[test]
    fn test_tvl_consistent_across_deposit_withdraw_sequences() {
        let env = setup_env();
        let id = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id, || {
            // Series of deposits and withdrawals
            apply_delta(&env, TvlSource::StakeVault, 1000).unwrap();
            apply_delta(&env, TvlSource::LiquidityPool, 500).unwrap();
            apply_delta(&env, TvlSource::StakeVault, -200).unwrap();
            apply_delta(&env, TvlSource::Escrow, 100).unwrap();
            apply_delta(&env, TvlSource::LiquidityPool, -50).unwrap();
            apply_delta(&env, TvlSource::Escrow, -30).unwrap();

            let snap = get_protocol_tvl(&env);
            // stake_vault: 1000 - 200 = 800
            // liquidity_pool: 500 - 50 = 450
            // escrow: 100 - 30 = 70
            // total: 1320
            assert_eq!(snap.stake_vault, 800);
            assert_eq!(snap.liquidity_pool, 450);
            assert_eq!(snap.escrow, 70);
            assert_eq!(
                snap.total,
                snap.stake_vault + snap.liquidity_pool + snap.escrow
            );
            assert_eq!(snap.total, 1320);
        });
    }

    #[test]
    fn test_full_withdrawal_returns_zero() {
        let env = setup_env();
        let id = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id, || {
            apply_delta(&env, TvlSource::StakeVault, 500).unwrap();
            apply_delta(&env, TvlSource::StakeVault, -500).unwrap();
            let snap = get_protocol_tvl(&env);
            assert_eq!(snap.stake_vault, 0);
            assert_eq!(snap.total, 0);
        });
    }

    #[test]
    fn test_convert_to_base_units_truncates_toward_zero_on_scale_down() {
        // A USDC-like asset with 9 decimals scaled down to 7 base decimals:
        // 1_234_567_890 raw units (1.23456789_0) -> 12_345_678 base units
        // (1.2345678), dropping the trailing "9" (0.9 of a base unit).
        assert_eq!(convert_to_base_units(1_234_567_890, 9).unwrap(), 12_345_678);
        // Negative amounts truncate toward zero too (never away from it),
        // so the direction never flips based on sign.
        assert_eq!(convert_to_base_units(-1_234_567_890, 9).unwrap(), -12_345_678);
    }

    #[test]
    fn test_convert_to_base_units_scales_up_exactly() {
        // A 2-decimal asset scaled up to 7 base decimals is an exact
        // multiplication, no precision is lost.
        assert_eq!(convert_to_base_units(100, 2).unwrap(), 100 * 10i128.pow(5));
    }

    #[test]
    fn test_convert_to_base_units_same_decimals_is_identity() {
        assert_eq!(convert_to_base_units(42, BASE_DECIMALS).unwrap(), 42);
    }

    #[test]
    fn test_multi_asset_tvl_stable_across_input_ordering() {
        // Two assets with different decimals, deposited into different
        // sources in one order, then the mirror order in a second contract
        // instance. The aggregate TVL must be identical either way.
        let env = setup_env();

        let id_a = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id_a, || {
            apply_asset_delta(&env, TvlSource::StakeVault, 1_234_567_890, 9).unwrap();
            apply_asset_delta(&env, TvlSource::LiquidityPool, 100, 2).unwrap();
        });
        let total_a = env.as_contract(&id_a, || get_protocol_tvl(&env).total);

        let id_b = env.register(crate::AnalyticsContract, ());
        env.as_contract(&id_b, || {
            apply_asset_delta(&env, TvlSource::LiquidityPool, 100, 2).unwrap();
            apply_asset_delta(&env, TvlSource::StakeVault, 1_234_567_890, 9).unwrap();
        });
        let total_b = env.as_contract(&id_b, || get_protocol_tvl(&env).total);

        assert_eq!(total_a, total_b);
        assert_eq!(total_a, 12_345_678 + 100 * 10i128.pow(5));
    }
}
