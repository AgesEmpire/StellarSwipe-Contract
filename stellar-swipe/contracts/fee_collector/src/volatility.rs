/// Issue: Dynamic fee bands based on market volatility.
///
/// Fees switch between configured bands according to volatility thresholds.
/// Logic is deterministic: given the same volatility signal the same band is
/// always selected.
use soroban_sdk::{contracttype, Env, Vec};

use crate::storage::get_congestion_signal;

/// A single volatility-to-fee band mapping.
/// When the current volatility signal (multiplier_bps) >= `volatility_threshold_bps`,
/// the fee rate for this band applies.
#[contracttype]
#[derive(Clone, Debug)]
pub struct VolatilityBand {
    /// Minimum multiplier_bps that activates this band (inclusive).
    pub volatility_threshold_bps: u32,
    /// Fee rate in basis points applied when this band is active.
    pub fee_rate_bps: u32,
}

/// Ordered list of volatility bands (ascending threshold order).
#[contracttype]
#[derive(Clone, Debug)]
pub struct VolatilityBandConfig {
    pub bands: Vec<VolatilityBand>,
    /// Fee rate used when no band threshold is met (lowest volatility).
    pub base_fee_rate_bps: u32,
}

#[contracttype]
enum VolatilityKey {
    BandConfig,
}

pub fn set_volatility_bands(env: &Env, config: &VolatilityBandConfig) {
    env.storage()
        .instance()
        .set(&VolatilityKey::BandConfig, config);
}

pub fn get_volatility_band_config(env: &Env) -> Option<VolatilityBandConfig> {
    env.storage()
        .instance()
        .get(&VolatilityKey::BandConfig)
}

/// Compute the fee rate for the current volatility signal.
/// Selects the highest-threshold band whose threshold <= current multiplier_bps.
/// Falls back to `base_fee_rate_bps` when no band matches.
pub fn fee_rate_for_current_volatility(env: &Env) -> Option<u32> {
    let config = get_volatility_band_config(env)?;
    let current_multiplier = get_congestion_signal(env)
        .map(|s| s.multiplier_bps)
        .unwrap_or(10_000); // default 1x

    let mut selected = config.base_fee_rate_bps;
    for i in 0..config.bands.len() {
        let band = config.bands.get(i).unwrap();
        if current_multiplier >= band.volatility_threshold_bps {
            selected = band.fee_rate_bps;
        }
    }
    Some(selected)
}
