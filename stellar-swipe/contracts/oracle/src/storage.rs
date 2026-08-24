//! Oracle storage layer — optimized layout
//!
//! Key improvements over previous design:
//!  - `PriceFeed` packs price + timestamp into one storage entry (was 2 keys)
//!  - `AvailablePairs` replaced by a compact `Vec<AssetPair>` (was `Map<AssetPair,bool>`)
//!  - Conversion cache TTL aligned to 5 min (unchanged), but struct is leaner
//!  - All persistent keys share a single TTL constant to avoid drift

use soroban_sdk::{contracttype, Env, Vec};
use stellar_swipe_common::{Asset, AssetPair};

use crate::errors::OracleError;

pub const DAY_IN_LEDGERS: u32 = 17_280; // ~24 h at 5 s/ledger
const PAIRS_TTL: u32 = DAY_IN_LEDGERS * 30; // pairs list lives 30 days

// ── Packed feed (replaces two separate Price + PriceTimestamp keys) ──────────

/// Single storage entry holding both the price and its timestamp.
/// Packing both fields eliminates one storage read/write per price update.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceFeed {
    pub price: i128,
    pub timestamp: u64,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub enum StorageKey {
    BaseCurrency,
    /// Replaces separate `Price` + `PriceTimestamp` keys — 50 % fewer key lookups
    Feed(AssetPair),
    /// Compact list instead of `Map<AssetPair, bool>` — avoids map overhead
    PairsList,
    ConversionCache(Asset, Asset),
    /// Native decimal precision for an asset pair (e.g. 6 for USDC, 7 for XLM).
    FeedDecimals(AssetPair),
}

// ── Cached conversion ─────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct CachedConversion {
    pub rate: i128,
    pub timestamp: u64,
}

// ── Base currency ─────────────────────────────────────────────────────────────

pub fn get_base_currency(env: &Env) -> Asset {
    env.storage()
        .persistent()
        .get(&StorageKey::BaseCurrency)
        .unwrap_or_else(|| default_base_currency(env))
}

pub fn set_base_currency(env: &Env, asset: Asset) {
    env.storage()
        .persistent()
        .set(&StorageKey::BaseCurrency, &asset);
    env.storage().persistent().extend_ttl(
        &StorageKey::BaseCurrency,
        DAY_IN_LEDGERS,
        DAY_IN_LEDGERS,
    );
}

// ── Price feed (packed) ───────────────────────────────────────────────────────

/// Returns the packed feed or `PriceNotFound`.
pub fn get_feed(env: &Env, pair: &AssetPair) -> Result<PriceFeed, OracleError> {
    env.storage()
        .persistent()
        .get(&StorageKey::Feed(pair.clone()))
        .ok_or(OracleError::PriceNotFound)
}

/// Convenience: price only.
pub fn get_price(env: &Env, pair: &AssetPair) -> Result<i128, OracleError> {
    get_feed(env, pair).map(|f| f.price)
}

/// Write price + timestamp in a single storage set (was 2 sets).
pub fn set_price(env: &Env, pair: &AssetPair, price: i128) {
    let feed = PriceFeed {
        price,
        timestamp: env.ledger().timestamp(),
    };
    let key = StorageKey::Feed(pair.clone());
    env.storage().persistent().set(&key, &feed);
    env.storage()
        .persistent()
        .extend_ttl(&key, DAY_IN_LEDGERS, DAY_IN_LEDGERS);
}

// ── Available pairs (compact Vec) ────────────────────────────────────────────

pub fn get_available_pairs(env: &Env) -> Vec<AssetPair> {
    env.storage()
        .persistent()
        .get(&StorageKey::PairsList)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn add_available_pair(env: &Env, pair: AssetPair) {
    let mut pairs = get_available_pairs(env);
    // Deduplicate without a map — pairs list is small (< 100 pairs expected)
    for i in 0..pairs.len() {
        if pairs.get(i).unwrap() == pair {
            return;
        }
    }
    pairs.push_back(pair);
    env.storage()
        .persistent()
        .set(&StorageKey::PairsList, &pairs);
    env.storage()
        .persistent()
        .extend_ttl(&StorageKey::PairsList, PAIRS_TTL, PAIRS_TTL);
}

// ── Conversion cache ──────────────────────────────────────────────────────────

pub fn get_cached_conversion(env: &Env, from: &Asset, to: &Asset) -> Option<CachedConversion> {
    let key = StorageKey::ConversionCache(from.clone(), to.clone());
    let cached: Option<CachedConversion> = env.storage().temporary().get(&key);
    if let Some(ref c) = cached {
        if env.ledger().timestamp().saturating_sub(c.timestamp) < 300 {
            return cached;
        }
    }
    None
}

pub fn set_cached_conversion(env: &Env, from: &Asset, to: &Asset, rate: i128) {
    let key = StorageKey::ConversionCache(from.clone(), to.clone());
    let cached = CachedConversion {
        rate,
        timestamp: env.ledger().timestamp(),
    };
    env.storage().temporary().set(&key, &cached);
    env.storage().temporary().extend_ttl(&key, 60, 60);
}

// ── Feed decimals ─────────────────────────────────────────────────────────────

/// Store the native decimal precision for an asset pair.
/// `decimals` is the number of decimal places in the raw price value
/// (e.g. 6 for USDC-priced feeds, 7 for XLM-denominated feeds).
pub fn set_feed_decimals(env: &Env, pair: &AssetPair, decimals: u32) {
    let key = StorageKey::FeedDecimals(pair.clone());
    env.storage().persistent().set(&key, &decimals);
    env.storage()
        .persistent()
        .extend_ttl(&key, DAY_IN_LEDGERS, DAY_IN_LEDGERS);
}

/// Retrieve stored decimal precision for an asset pair.
/// Returns `None` if no decimals have been configured.
pub fn get_feed_decimals(env: &Env, pair: &AssetPair) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&StorageKey::FeedDecimals(pair.clone()))
}

/// Rescale `raw_price` from `from_decimals` to `to_decimals`.
///
/// Uses integer-only arithmetic to stay inside `no_std`:
/// - If `to_decimals > from_decimals` the price is multiplied by 10^(diff).
/// - If `to_decimals < from_decimals` the price is divided by 10^(diff).
/// - Returns `None` on overflow.
pub fn rescale_price(raw_price: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    if from_decimals == to_decimals {
        return Some(raw_price);
    }
    if to_decimals > from_decimals {
        let diff = to_decimals - from_decimals;
        let factor = pow10(diff)?;
        raw_price.checked_mul(factor)
    } else {
        let diff = from_decimals - to_decimals;
        let factor = pow10(diff)?;
        Some(raw_price / factor)
    }
}

fn pow10(exp: u32) -> Option<i128> {
    let mut result: i128 = 1;
    for _ in 0..exp {
        result = result.checked_mul(10)?;
    }
    Some(result)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn default_base_currency(env: &Env) -> Asset {
    Asset {
        code: soroban_sdk::String::from_str(env, "XLM"),
        issuer: None,
    }
}
