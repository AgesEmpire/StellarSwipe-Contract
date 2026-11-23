//! Dynamic fee optimization based on real-time network conditions.
//!
//! # Overview
//!
//! Fee tiers are recalculated on each transaction based on three inputs:
//!
//! 1. **Network congestion** – tracked as the rolling transaction count over
//!    the last ledger window (pushed by an off-chain keeper or derived from
//!    the previous ledger sequence delta).
//! 2. **Transaction volume** – the aggregate USD-equivalent volume processed
//!    by the fee collector in the current epoch; higher volume unlocks lower
//!    fee tiers (maker discounts).
//! 3. **Fee tier** – a user's historical volume bracket that earns a discount.
//!
//! # Fee Formula
//!
//! ```text
//! effective_fee_bps = base_fee_bps
//!                   × congestion_multiplier(network_load_pct)
//!                   × volume_discount_factor(epoch_volume)
//!                   × tier_discount_factor(user_tier)
//! ```
//!
//! All multipliers are expressed in basis-points (10 000 = 1×).
//!
//! # Fee Predictability
//!
//! [`predict_fee`] returns the fee that *would* be charged given the current
//! network state, without mutating state.  Front-ends must call this before
//! presenting a "you will pay" amount to the user.
//!
//! # Fee Transparency
//!
//! Every fee calculation emits a `fee_optimized` event that records the
//! components (base, congestion multiplier, volume discount, tier discount)
//! so that off-chain tooling can audit the fee structure.

#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::errors::ContractError;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default base fee in basis points (30 bps = 0.30 %).
pub const DEFAULT_BASE_FEE_BPS: u32 = 30;

/// Maximum configurable base fee (500 bps = 5 %).
pub const MAX_BASE_FEE_BPS: u32 = 500;

/// Denominator for all multiplier arithmetic (10 000 = 1×).
pub const BPS_DENOM: u32 = 10_000;

/// At 0–50 % network load: no surcharge (multiplier = 1×).
pub const CONGESTION_LOW_THRESHOLD_PCT: u32 = 50;

/// At 51–80 % network load: 25 % surcharge (multiplier = 1.25×).
pub const CONGESTION_MED_THRESHOLD_PCT: u32 = 80;

/// At > 80 % network load: 75 % surcharge (multiplier = 1.75×).
pub const CONGESTION_HIGH_THRESHOLD_PCT: u32 = 100;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Network condition snapshot pushed by the monitoring layer.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConditions {
    /// Percentage of theoretical throughput currently in use (0–100).
    pub load_pct: u32,
    /// Median ledger close time for the last 10 ledgers, in milliseconds.
    /// Values above 6 000 ms indicate congestion regardless of `load_pct`.
    pub median_close_time_ms: u32,
    /// Ledger sequence at which these conditions were observed.
    pub observed_at_ledger: u32,
}

/// A user's fee tier, based on 30-day rolling volume.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeeTier {
    /// < 1 000 USD equivalent.
    Standard,
    /// 1 000 – 50 000 USD equivalent.
    Silver,
    /// 50 000 – 500 000 USD equivalent.
    Gold,
    /// > 500 000 USD equivalent.
    Platinum,
}

/// Configuration for the dynamic fee optimizer.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeOptimizerConfig {
    /// Floor fee regardless of discounts (bps).
    pub floor_fee_bps: u32,
    /// Ceiling fee regardless of surcharges (bps).
    pub ceiling_fee_bps: u32,
    /// Base fee before any adjustment (bps).
    pub base_fee_bps: u32,
    /// Congestion surcharge at medium load (bps added to base).
    pub congestion_med_surcharge_bps: u32,
    /// Congestion surcharge at high load (bps added to base).
    pub congestion_high_surcharge_bps: u32,
    /// Silver tier discount (bps deducted).
    pub silver_discount_bps: u32,
    /// Gold tier discount (bps deducted).
    pub gold_discount_bps: u32,
    /// Platinum tier discount (bps deducted).
    pub platinum_discount_bps: u32,
    /// Volume discount threshold 1 in protocol units.
    pub volume_tier1_threshold: u64,
    /// Volume discount tier 1 (bps deducted when epoch volume > tier1 threshold).
    pub volume_tier1_discount_bps: u32,
    /// Volume discount threshold 2 in protocol units.
    pub volume_tier2_threshold: u64,
    /// Volume discount tier 2 (bps deducted when epoch volume > tier2 threshold).
    pub volume_tier2_discount_bps: u32,
    /// Maximum ledger age for a network-condition snapshot before it is stale.
    pub max_condition_age_ledgers: u32,
}

impl Default for FeeOptimizerConfig {
    fn default() -> Self {
        FeeOptimizerConfig {
            floor_fee_bps: 5,
            ceiling_fee_bps: 300,
            base_fee_bps: DEFAULT_BASE_FEE_BPS,
            congestion_med_surcharge_bps: 10,
            congestion_high_surcharge_bps: 40,
            silver_discount_bps: 3,
            gold_discount_bps: 8,
            platinum_discount_bps: 15,
            volume_tier1_threshold: 1_000_000,
            volume_tier1_discount_bps: 2,
            volume_tier2_threshold: 10_000_000,
            volume_tier2_discount_bps: 5,
            max_condition_age_ledgers: 100,
        }
    }
}

/// Detailed breakdown of a computed fee.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeBreakdown {
    /// Final effective fee in basis points.
    pub effective_bps: u32,
    /// Base fee before adjustments.
    pub base_bps: u32,
    /// Congestion surcharge applied (0 if no congestion).
    pub congestion_surcharge_bps: u32,
    /// Tier discount deducted (0 for Standard tier).
    pub tier_discount_bps: u32,
    /// Volume discount deducted (0 if below threshold).
    pub volume_discount_bps: u32,
    /// Fee tier of the user at calculation time.
    pub user_tier: FeeTier,
    /// Network load percentage used for this calculation.
    pub network_load_pct: u32,
}

// ── Storage ───────────────────────────────────────────────────────────────────

#[contracttype]
pub enum FeeOptimizerKey {
    Config,
    NetworkConditions,
    EpochVolume,
}

pub fn get_config(env: &Env) -> FeeOptimizerConfig {
    env.storage()
        .instance()
        .get(&FeeOptimizerKey::Config)
        .unwrap_or_default()
}

pub fn set_config(env: &Env, config: &FeeOptimizerConfig) {
    env.storage()
        .instance()
        .set(&FeeOptimizerKey::Config, config);
}

/// Return the latest network conditions, or `None` if none have been recorded.
pub fn get_network_conditions(env: &Env) -> Option<NetworkConditions> {
    env.storage()
        .instance()
        .get(&FeeOptimizerKey::NetworkConditions)
}

/// Update network conditions (called by an admin-approved monitor keeper).
pub fn update_network_conditions(
    env: &Env,
    conditions: NetworkConditions,
) -> Result<(), ContractError> {
    if conditions.load_pct > 100 {
        return Err(ContractError::NetworkConditionInvalid);
    }
    env.storage()
        .instance()
        .set(&FeeOptimizerKey::NetworkConditions, &conditions);
    env.events().publish(
        (Symbol::new(env, "net_conditions"),),
        (
            conditions.load_pct,
            conditions.median_close_time_ms,
            conditions.observed_at_ledger,
        ),
    );
    Ok(())
}

/// Accumulated epoch volume (denominated in protocol base units).
pub fn get_epoch_volume(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&FeeOptimizerKey::EpochVolume)
        .unwrap_or(0)
}

pub fn add_epoch_volume(env: &Env, amount: u64) {
    let vol = get_epoch_volume(env).saturating_add(amount);
    env.storage()
        .instance()
        .set(&FeeOptimizerKey::EpochVolume, &vol);
}

pub fn reset_epoch_volume(env: &Env) {
    env.storage()
        .instance()
        .set(&FeeOptimizerKey::EpochVolume, &0u64);
}

// ── Core Fee Calculation ──────────────────────────────────────────────────────

/// Return `true` when the stored network conditions are still fresh enough to
/// use; stale conditions fall back to no congestion surcharge.
fn conditions_are_fresh(env: &Env, conds: &NetworkConditions, cfg: &FeeOptimizerConfig) -> bool {
    let age = env
        .ledger()
        .sequence()
        .saturating_sub(conds.observed_at_ledger);
    age <= cfg.max_condition_age_ledgers
}

/// Compute the congestion surcharge in basis points given `load_pct`.
fn congestion_surcharge(load_pct: u32, cfg: &FeeOptimizerConfig) -> u32 {
    if load_pct <= CONGESTION_LOW_THRESHOLD_PCT {
        0
    } else if load_pct <= CONGESTION_MED_THRESHOLD_PCT {
        cfg.congestion_med_surcharge_bps
    } else {
        cfg.congestion_high_surcharge_bps
    }
}

/// Compute the tier discount in basis points.
fn tier_discount(tier: FeeTier, cfg: &FeeOptimizerConfig) -> u32 {
    match tier {
        FeeTier::Standard => 0,
        FeeTier::Silver => cfg.silver_discount_bps,
        FeeTier::Gold => cfg.gold_discount_bps,
        FeeTier::Platinum => cfg.platinum_discount_bps,
    }
}

/// Compute the volume discount in basis points given the current epoch volume.
fn volume_discount(epoch_volume: u64, cfg: &FeeOptimizerConfig) -> u32 {
    if epoch_volume >= cfg.volume_tier2_threshold {
        cfg.volume_tier2_discount_bps
    } else if epoch_volume >= cfg.volume_tier1_threshold {
        cfg.volume_tier1_discount_bps
    } else {
        0
    }
}

/// Core calculation: apply all adjustments and clamp to [floor, ceiling].
fn compute_fee_bps(
    base_bps: u32,
    surcharge: u32,
    tier_disc: u32,
    vol_disc: u32,
    cfg: &FeeOptimizerConfig,
) -> u32 {
    let adjusted = base_bps
        .saturating_add(surcharge)
        .saturating_sub(tier_disc)
        .saturating_sub(vol_disc);
    adjusted.max(cfg.floor_fee_bps).min(cfg.ceiling_fee_bps)
}

/// Calculate the effective fee in basis points for `tier` given the current
/// contract state. Mutates epoch volume by `trade_volume` **only** when
/// `record_volume` is `true`.
///
/// Emits a `fee_optimized` event with the full breakdown for auditability.
pub fn calculate_fee(
    env: &Env,
    tier: FeeTier,
    trade_volume: u64,
    record_volume: bool,
) -> FeeBreakdown {
    let cfg = get_config(env);
    let epoch_volume = get_epoch_volume(env);

    let (load_pct, surcharge_bps) = match get_network_conditions(env) {
        Some(ref conds) if conditions_are_fresh(env, conds, &cfg) => {
            let s = congestion_surcharge(conds.load_pct, &cfg);
            (conds.load_pct, s)
        }
        _ => (0u32, 0u32), // no fresh data → no surcharge
    };

    let tier_disc = tier_discount(tier, &cfg);
    let vol_disc = volume_discount(epoch_volume, &cfg);
    let effective_bps =
        compute_fee_bps(cfg.base_fee_bps, surcharge_bps, tier_disc, vol_disc, &cfg);

    if record_volume {
        add_epoch_volume(env, trade_volume);
    }

    let breakdown = FeeBreakdown {
        effective_bps,
        base_bps: cfg.base_fee_bps,
        congestion_surcharge_bps: surcharge_bps,
        tier_discount_bps: tier_disc,
        volume_discount_bps: vol_disc,
        user_tier: tier,
        network_load_pct: load_pct,
    };

    env.events().publish(
        (symbol_short!("fee_opt"),),
        (
            effective_bps,
            surcharge_bps,
            tier_disc,
            vol_disc,
            load_pct,
        ),
    );

    breakdown
}

/// Predict the fee that *would* be charged right now for `tier` and
/// `trade_volume` without altering any state.  Safe to call from read-only
/// contexts (e.g., front-end simulation).
pub fn predict_fee(env: &Env, tier: FeeTier, trade_volume: u64) -> FeeBreakdown {
    calculate_fee(env, tier, trade_volume, false)
}

/// Return the validated and updated config after an admin write.
///
/// The caller must be the contract admin (fetched from storage via
/// `crate::storage::get_admin`). Using the same pattern as other
/// admin-gated entry points in `fee_collector/src/lib.rs`.
pub fn update_config(
    env: &Env,
    caller: &soroban_sdk::Address,
    new_config: FeeOptimizerConfig,
) -> Result<FeeOptimizerConfig, ContractError> {
    let admin = crate::storage::get_admin(env);
    if *caller != admin {
        return Err(ContractError::Unauthorized);
    }
    caller.require_auth();
    if new_config.base_fee_bps > MAX_BASE_FEE_BPS {
        return Err(ContractError::InvalidFeeConfiguration);
    }
    if new_config.floor_fee_bps > new_config.ceiling_fee_bps {
        return Err(ContractError::InvalidFeeConfiguration);
    }
    set_config(env, &new_config);
    env.events().publish(
        (Symbol::new(env, "fee_cfg_updated"),),
        (new_config.base_fee_bps, new_config.floor_fee_bps, new_config.ceiling_fee_bps),
    );
    Ok(new_config)
}
