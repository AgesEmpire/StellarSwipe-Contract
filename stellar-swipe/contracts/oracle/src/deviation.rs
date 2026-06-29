//! Cross-source price deviation alerting (Issue #671).
//!
//! On each `submit_pair_price` call, computes the max pairwise deviation
//! across all currently reporting sources and emits `price_deviation_alert`
//! when the deviation exceeds an admin-configured threshold — independently
//! of whether aggregation or fallback logic triggers.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};
use stellar_swipe_common::AssetPair;

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
enum DeviationKey {
    Threshold(AssetPair),
}

// ── Admin ─────────────────────────────────────────────────────────────────────

/// Set admin-configurable deviation alert threshold in basis points (100 = 1 %).
pub fn set_deviation_threshold(env: &Env, pair: &AssetPair, threshold_bps: u32) {
    env.storage()
        .persistent()
        .set(&DeviationKey::Threshold(pair.clone()), &threshold_bps);
}

/// Get deviation alert threshold in basis points. Returns 0 if not configured.
pub fn get_deviation_threshold(env: &Env, pair: &AssetPair) -> u32 {
    env.storage()
        .persistent()
        .get(&DeviationKey::Threshold(pair.clone()))
        .unwrap_or(0)
}

// ── Check & emit ──────────────────────────────────────────────────────────────

/// Compute max deviation across fresh price sources and emit
/// `price_deviation_alert` if it exceeds the configured threshold.
///
/// `prices` is the slice of (source, price) from all currently reporting sources.
pub fn check_deviation(env: &Env, pair: &AssetPair, sources: &Vec<(Address, i128)>) {
    let threshold_bps = get_deviation_threshold(env, pair);
    if threshold_bps == 0 || sources.is_empty() {
        return;
    }

    // Find min and max price across all sources
    let mut min_price = i128::MAX;
    let mut max_price = i128::MIN;

    for i in 0..sources.len() {
        let (_, price) = sources.get(i).unwrap();
        if price < min_price {
            min_price = price;
        }
        if price > max_price {
            max_price = price;
        }
    }

    if min_price <= 0 {
        return;
    }

    let deviation_bps = ((max_price - min_price) * 10_000) / min_price;
    if deviation_bps as u32 > threshold_bps {
        // Collect offending source identifiers (all sources contributing to max deviation)
        let mut offending: Vec<Address> = Vec::new(env);
        for i in 0..sources.len() {
            let (source, price) = sources.get(i).unwrap();
            let src_deviation_bps = ((price - min_price).abs() * 10_000) / min_price;
            if src_deviation_bps as u32 > threshold_bps {
                offending.push_back(source);
            }
        }

        env.events().publish(
            (Symbol::new(env, "price_deviation_alert"),),
            (
                pair.clone(),
                deviation_bps as u32,
                threshold_bps,
                offending,
            ),
        );
    }
}
