//! Price conversion system for multi-asset portfolio aggregation

use crate::errors::OracleError;
use crate::storage::{get_base_currency, get_feed_decimals, get_price, rescale_price};
use shared::math::normalize_amount;
use soroban_sdk::{contracttype, vec, Env, Map, Vec};
use stellar_swipe_common::{Asset, AssetPair};

const MAX_PATH_LENGTH: u32 = 3;

#[contracttype]
#[derive(Clone, Debug)]
pub struct ConversionPath {
    pub assets: Vec<Asset>,
    pub total_hops: u32,
}

/// Convert amount to base currency using direct or path-based conversion
pub fn convert_to_base(env: &Env, amount: i128, asset: Asset) -> Result<i128, OracleError> {
    let base = get_base_currency(env);

    if asset == base {
        return Ok(amount);
    }

    // Try direct conversion first
    if let Ok(result) = convert_direct(env, amount, &asset, &base) {
        return Ok(result);
    }

    // Fall back to path-based conversion
    convert_via_path(env, amount, asset, base)
}

/// Normalize a raw `price` from `from_decimals` to canonical 7-decimal precision.
///
/// Returns `None` if the rescaling overflows `i128`.
pub fn normalize_price(price: i128, from_decimals: u32) -> Option<i128> {
    shared::math::normalize_amount(price, from_decimals, 7)
}

/// Read the live price for `pair` rescaled to canonical 7-decimal precision.
///
/// Mirrors the `get_normalized_price` contract entrypoint: the raw stored feed
/// price is rescaled from its configured native decimals (defaulting to 7 when
/// unconfigured) to the canonical precision so that feeds stored with
/// different native precisions produce deterministic results.
fn get_normalized_price(env: &Env, pair: &AssetPair) -> Result<i128, OracleError> {
    let raw_price = get_price(env, pair)?;
    let from_decimals = get_feed_decimals(env, pair).unwrap_or(7);
    rescale_price(raw_price, from_decimals, 7).ok_or(OracleError::ConversionOverflow)
}

/// Direct conversion: asset → base
///
/// Prices are normalized to canonical 7-decimal precision before scaling so
/// that feeds stored with different native precisions produce deterministic
/// results.
fn convert_direct(env: &Env, amount: i128, from: &Asset, to: &Asset) -> Result<i128, OracleError> {
    let pair = AssetPair {
        base: from.clone(),
        quote: to.clone(),
    };
    let price = get_normalized_price(env, &pair)?;

    let product = amount
        .checked_mul(price)
        .ok_or(OracleError::ConversionOverflow)?;

    normalize_amount(product, 14, 7).ok_or(OracleError::ConversionOverflow)
}

/// Path-based conversion: asset → intermediate(s) → base
fn convert_via_path(env: &Env, amount: i128, from: Asset, to: Asset) -> Result<i128, OracleError> {
    let path = find_conversion_path(env, &from, &to)?;

    let mut current_amount = amount;
    let mut current_asset = from;

    for i in 1..path.assets.len() {
        let next_asset = path.assets.get(i).ok_or(OracleError::InvalidPath)?;
        current_amount = convert_direct(env, current_amount, &current_asset, &next_asset)?;
        current_asset = next_asset;
    }

    Ok(current_amount)
}

/// Find shortest conversion path using BFS
fn find_conversion_path(
    env: &Env,
    from: &Asset,
    to: &Asset,
) -> Result<ConversionPath, OracleError> {
    let available_pairs = get_available_pairs(env);

    let mut queue: Vec<Vec<Asset>> = vec![env];
    let mut start_path = vec![env];
    start_path.push_back(from.clone());
    queue.push_back(start_path);

    let mut visited: Map<Asset, bool> = Map::new(env);
    visited.set(from.clone(), true);

    while !queue.is_empty() {
        let path = queue.get(0).ok_or(OracleError::NoConversionPath)?;
        queue.remove(0);

        if path.len() > MAX_PATH_LENGTH {
            continue;
        }

        let current = path.last().ok_or(OracleError::InvalidPath)?;

        if current == *to {
            let total_hops = path.len().saturating_sub(1);
            return Ok(ConversionPath {
                assets: path,
                total_hops,
            });
        }

        // Explore neighbors
        for pair in available_pairs.iter() {
            let next = if pair.base == current {
                Some(pair.quote.clone())
            } else if pair.quote == current {
                Some(pair.base.clone())
            } else {
                None
            };

            if let Some(next_asset) = next {
                if !visited.contains_key(next_asset.clone()) {
                    visited.set(next_asset.clone(), true);
                    let mut new_path = path.clone();
                    new_path.push_back(next_asset);
                    queue.push_back(new_path);
                }
            }
        }
    }

    Err(OracleError::NoConversionPath)
}

/// Get all available trading pairs from storage
fn get_available_pairs(env: &Env) -> Vec<AssetPair> {
    let pairs_map = crate::storage::get_available_pairs(env);
    let mut pairs = vec![env];
    for pair in pairs_map.iter() {
        pairs.push_back(pair);
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{set_base_currency, set_price};
    use soroban_sdk::{testutils::Address as _, Address, Env, String};

    fn xlm(env: &Env) -> Asset {
        Asset {
            code: String::from_str(env, "XLM"),
            issuer: None,
        }
    }

    fn usdc(env: &Env) -> Asset {
        Asset {
            code: String::from_str(env, "USDC"),
            issuer: Some(Address::generate(env)),
        }
    }

    fn with_oracle_contract<F, R>(f: F) -> R
    where
        F: FnOnce(&Env) -> R,
    {
        let env = Env::default();
        let contract_id = env.register_contract(None, crate::OracleContract);
        env.as_contract(&contract_id, || f(&env))
    }

    #[test]
    fn test_convert_same_asset() {
        with_oracle_contract(|env| {
            let xlm = xlm(env);
            set_base_currency(env, xlm.clone());

            let result = convert_to_base(env, 1000_0000000, xlm).unwrap();
            assert_eq!(result, 1000_0000000);
        });
    }

    #[test]
    fn test_direct_conversion() {
        with_oracle_contract(|env| {
            let xlm = xlm(env);
            let usdc = usdc(env);

            set_base_currency(env, xlm.clone());

            // 1 USDC = 10 XLM
            let pair = AssetPair {
                base: usdc.clone(),
                quote: xlm.clone(),
            };
            set_price(env, &pair, 10 * 10_000_000i128); // 10 XLM per USDC (7-decimal rate)

            // Convert 100 USDC to XLM
            let result = convert_to_base(env, 100_0000000, usdc).unwrap();
            assert_eq!(result, 1000_0000000); // 100 * 10 = 1000 XLM
        });
    }
}
