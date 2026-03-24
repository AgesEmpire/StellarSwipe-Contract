//! Oracle storage layer

use crate::errors::OracleError;
use common::{Asset, AssetPair};
use soroban_sdk::{contracttype, Env, Map};

const DAY_IN_LEDGERS: u32 = 17280; // ~24 hours

#[contracttype]
#[derive(Clone, Debug)]
pub enum StorageKey {
    BaseCurrency,
    Price(AssetPair),
    PriceTimestamp(AssetPair),
    AvailablePairs,
    ConversionCache(Asset, Asset),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CachedConversion {
    pub rate: i128,
    pub timestamp: u64,
}

/// Get base currency (default: XLM)
pub fn get_base_currency(env: &Env) -> Asset {
    env.storage()
        .persistent()
        .get(&StorageKey::BaseCurrency)
        .unwrap_or_else(|| default_base_currency(env))
}

/// Set base currency
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

/// Get price for asset pair
pub fn get_price(env: &Env, pair: &AssetPair) -> Result<i128, OracleError> {
    let key = StorageKey::Price(pair.clone());
    env.storage()
        .persistent()
        .get(&key)
        .ok_or(OracleError::PriceNotFound)
}

/// Set price for asset pair
pub fn set_price(env: &Env, pair: &AssetPair, price: i128) {
    let key = StorageKey::Price(pair.clone());
    let ts_key = StorageKey::PriceTimestamp(pair.clone());

    env.storage().persistent().set(&key, &price);
    env.storage()
        .persistent()
        .set(&ts_key, &env.ledger().timestamp());

    env.storage()
        .persistent()
        .extend_ttl(&key, DAY_IN_LEDGERS, DAY_IN_LEDGERS);
    env.storage()
        .persistent()
        .extend_ttl(&ts_key, DAY_IN_LEDGERS, DAY_IN_LEDGERS);
}

/// Get cached conversion rate
pub fn get_cached_conversion(env: &Env, from: &Asset, to: &Asset) -> Option<CachedConversion> {
    let key = StorageKey::ConversionCache(from.clone(), to.clone());
    let cached: Option<CachedConversion> = env.storage().temporary().get(&key);

    if let Some(ref c) = cached {
        // Cache valid for 5 minutes (60 ledgers)
        if env.ledger().timestamp() - c.timestamp < 300 {
            return cached;
        }
    }
    None
}

/// Set cached conversion rate
pub fn set_cached_conversion(env: &Env, from: &Asset, to: &Asset, rate: i128) {
    let key = StorageKey::ConversionCache(from.clone(), to.clone());
    let cached = CachedConversion {
        rate,
        timestamp: env.ledger().timestamp(),
    };
    env.storage().temporary().set(&key, &cached);
    env.storage().temporary().extend_ttl(&key, 60, 60); // 5 minutes
}

/// Get available trading pairs
pub fn get_available_pairs(env: &Env) -> Map<AssetPair, bool> {
    env.storage()
        .persistent()
        .get(&StorageKey::AvailablePairs)
        .unwrap_or_else(|| Map::new(env))
}

/// Add available trading pair
pub fn add_available_pair(env: &Env, pair: AssetPair) {
    let mut pairs = get_available_pairs(env);
    pairs.set(pair, true);
    env.storage()
        .persistent()
        .set(&StorageKey::AvailablePairs, &pairs);
    env.storage().persistent().extend_ttl(
        &StorageKey::AvailablePairs,
        DAY_IN_LEDGERS,
        DAY_IN_LEDGERS,
    );
}

fn default_base_currency(env: &Env) -> Asset {
    Asset {
        code: soroban_sdk::String::from_str(env, "XLM"),
        issuer: None,
    }
}
