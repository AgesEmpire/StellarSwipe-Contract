use crate::errors::OracleError;
use crate::sdex;
use crate::storage;
use stellar_swipe_common::{Asset, AssetPair};
use soroban_sdk::{vec, Env, Map, Vec};

const PRECISION: i128 = 10_000_000;
const MAX_HOPS: u32 = 3;
const SLIPPAGE_TOLERANCE_BPS: u32 = 500; // 5%

#[soroban_sdk::contracttype]
#[derive(Clone, Debug)]
pub struct LiquidityPath {
    pub hops: Vec<AssetPair>,
    pub total_liquidity: i128,
    pub estimated_slippage: u32,
    pub total_fees: u32,
}

/// Find the optimal liquidity path between two assets considering slippage and fees.
pub fn find_optimal_path(
    env: &Env,
    from: Asset,
    to: Asset,
    amount: i128,
) -> Result<LiquidityPath, OracleError> {
    // 1. Check cache first (5 minutes)
    if let Some(cached_path) = get_cached_path(env, &from, &to) {
        return Ok(cached_path);
    }

    // 2. Build liquidity graph from available pairs
    let graph = build_liquidity_graph(env);

    // 3. Find all paths up to MAX_HOPS (3)
    let paths = find_all_paths(env, &graph, &from, &to, MAX_HOPS);

    // 4. Calculate cost and find the best one
    let mut best_path: Option<LiquidityPath> = None;
    let mut min_cost = u32::MAX;

    for path in paths.iter() {
        if let Ok(liquidity_path) = calculate_path_cost(env, path, amount) {
            let cost = liquidity_path.estimated_slippage + liquidity_path.total_fees;
            if cost < min_cost {
                min_cost = cost;
                best_path = Some(liquidity_path);
            }
        }
    }

    let final_path = best_path.ok_or(OracleError::NoPathFound)?;

    // 5. Cache the result for 5 minutes
    cache_path(env, &from, &to, &final_path);

    Ok(final_path)
}

/// Calculate effective price through multiple hops.
pub fn calculate_multi_hop_price(env: &Env, path: LiquidityPath, amount: i128) -> i128 {
    let mut current_amount = amount;

    for hop in path.hops.iter() {
        let (price, slippage) = get_price_with_slippage(env, hop, current_amount);

        // current_amount = (current_amount * price / PRECISION) * (10000 - slippage) / 10000;
        let price_impact = current_amount
            .checked_mul(price)
            .and_then(|v| v.checked_div(PRECISION))
            .unwrap_or(0);

        current_amount = price_impact
            .checked_mul(10000 - slippage as i128)
            .and_then(|v| v.checked_div(10000))
            .unwrap_or(0);
    }

    current_amount
}

/// Estimate price and slippage for a single hop.
pub fn get_price_with_slippage(env: &Env, pair: AssetPair, amount: i128) -> (i128, u32) {
    // In a real implementation, this would query the order book.
    // For now, we try to get the price from storage and estimate slippage.

    let spot_price = storage::get_price(env, &pair).unwrap_or(PRECISION);

    // Simple slippage model based on trade size vs "virtual" liquidity.
    // In real scenario, we'd use sdex::calculate_vwap if we had orderbook data.
    // Here we assume a 0.1% slippage for every 10,000 units of amount beyond 1000.
    let base_slippage = 10; // 10 bps minimum
    let volume_slippage = if amount > 1000 * PRECISION {
        ((amount / PRECISION - 1000) / 10000) as u32 * 5 // 5 bps per 10k units
    } else {
        0
    };

    let total_slippage = base_slippage + volume_slippage;

    (spot_price, total_slippage)
}

// Internal helpers

fn build_liquidity_graph(env: &Env) -> Vec<AssetPair> {
    let pairs_map = storage::get_available_pairs(env);
    let mut pairs = Vec::new(env);
    for (pair, _) in pairs_map.iter() {
        pairs.push_back(pair);
    }
    pairs
}

fn find_all_paths(
    env: &Env,
    graph: &Vec<AssetPair>,
    from: &Asset,
    to: &Asset,
    max_hops: u32,
) -> Vec<Vec<AssetPair>> {
    let mut result = Vec::new(env);
    let mut current_path = Vec::new(env);
    let mut visited = Map::new(env);
    visited.set(from.clone(), true);

    dfs(
        env,
        graph,
        from,
        to,
        max_hops,
        &mut current_path,
        &mut visited,
        &mut result,
    );
    result
}

fn dfs(
    env: &Env,
    graph: &Vec<AssetPair>,
    current: &Asset,
    target: &Asset,
    remaining_hops: u32,
    current_path: &mut Vec<AssetPair>,
    visited: &mut Map<Asset, bool>,
    results: &mut Vec<Vec<AssetPair>>,
) {
    if remaining_hops == 0 {
        return;
    }

    for pair in graph.iter() {
        let next = if pair.base == *current {
            Some(pair.quote.clone())
        } else if pair.quote == *current {
            Some(pair.base.clone())
        } else {
            None
        };

        if let Some(next_asset) = next {
            if visited.contains_key(next_asset.clone()) {
                continue;
            }

            // Represent hop in direction of trade
            let hop = AssetPair {
                base: current.clone(),
                quote: next_asset.clone(),
            };

            current_path.push_back(hop);

            if next_asset == *target {
                results.push_back(current_path.clone());
            } else {
                visited.set(next_asset.clone(), true);
                dfs(
                    env,
                    graph,
                    &next_asset,
                    target,
                    remaining_hops - 1,
                    current_path,
                    visited,
                    results,
                );
                visited.remove(next_asset.clone()); // Backtrack
            }

            current_path.remove(current_path.len() - 1);
        }
    }
}

fn calculate_path_cost(
    env: &Env,
    path: Vec<AssetPair>,
    amount: i128,
) -> Result<LiquidityPath, OracleError> {
    let mut current_amount = amount;
    let mut total_slippage: u32 = 0;
    let mut total_fees: u32 = 0;
    let mut min_liquidity: i128 = i128::MAX;

    for hop in path.iter() {
        // Assume 0.3% fee per hop (30 bps)
        let fee_bps = 30;
        total_fees += fee_bps;

        let (price, slippage) = get_price_with_slippage(env, hop, current_amount);
        total_slippage += slippage;

        // Mock liquidity as 1M units for now
        let liquidity = 1_000_000 * PRECISION;
        if liquidity < min_liquidity {
            min_liquidity = liquidity;
        }

        current_amount = (current_amount * price / PRECISION) * (10000 - slippage as i128) / 10000;
    }

    if total_slippage > SLIPPAGE_TOLERANCE_BPS {
        return Err(OracleError::SlippageExceeded);
    }

    Ok(LiquidityPath {
        hops: path,
        total_liquidity: min_liquidity,
        estimated_slippage: total_slippage,
        total_fees,
    })
}

// Caching logic

#[soroban_sdk::contracttype]
#[derive(Clone, Debug)]
pub enum MultiHopKey {
    CachedPath(Asset, Asset),
}

fn get_cached_path(env: &Env, from: &Asset, to: &Asset) -> Option<LiquidityPath> {
    let key = MultiHopKey::CachedPath(from.clone(), to.clone());
    env.storage().temporary().get(&key)
}

fn cache_path(env: &Env, from: &Asset, to: &Asset, path: &LiquidityPath) {
    let key = MultiHopKey::CachedPath(from.clone(), to.clone());
    env.storage().temporary().set(&key, path);
    env.storage().temporary().extend_ttl(&key, 60, 60); // 5 minutes (assuming ~5s per ledger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{add_available_pair, set_base_currency, set_price};
    use soroban_sdk::{testutils::Address as _, Address, String};

    fn create_asset(env: &Env, code: &str) -> Asset {
        Asset {
            code: String::from_str(env, code),
            issuer: Some(Address::generate(env)),
        }
    }

    fn xlm(env: &Env) -> Asset {
        Asset {
            code: String::from_str(env, "XLM"),
            issuer: None,
        }
    }

    #[test]
    fn test_find_optimal_path_2_hops() {
        let env = Env::default();
        let token_a = create_asset(&env, "TOKENA");
        let xlm = xlm(&env);
        let token_b = create_asset(&env, "TOKENB");

        // Set up pairs: TOKENA/XLM and XLM/TOKENB
        let pair1 = AssetPair {
            base: token_a.clone(),
            quote: xlm.clone(),
        };
        let pair2 = AssetPair {
            base: xlm.clone(),
            quote: token_b.clone(),
        };

        set_price(&env, &pair1, 10 * PRECISION); // 1 TOKENA = 10 XLM
        set_price(&env, &pair2, 2 * PRECISION); // 1 XLM = 2 TOKENB

        add_available_pair(&env, pair1.clone());
        add_available_pair(&env, pair2.clone());

        let path =
            find_optimal_path(&env, token_a.clone(), token_b.clone(), 100 * PRECISION).unwrap();

        assert_eq!(path.hops.len(), 2);
        assert_eq!(path.estimated_slippage, 20); // 10 bps per hop

        let final_amount = calculate_multi_hop_price(&env, path, 100 * PRECISION);

        // Manual calculation:
        // 100 TOKENA -> XLM: 100 * 10 = 1000 XLM. Slippage 0.1% -> 999 XLM
        // 999 XLM -> TOKENB: 999 * 2 = 1998 TOKENB. Slippage 0.1% -> 1996.002 -> 1996
        // Precision is 10,000,000
        // 999 * 2 * 9990 / 10000 = 1998 * 0.999 = 1996.002
        assert_eq!(final_amount, 1996_0020000);
    }

    #[test]
    fn test_find_all_paths_max_hops() {
        let env = Env::default();
        let a = create_asset(&env, "A");
        let b = create_asset(&env, "B");
        let c = create_asset(&env, "C");
        let d = create_asset(&env, "D");

        let graph = vec![
            &env,
            AssetPair {
                base: a.clone(),
                quote: b.clone(),
            },
            AssetPair {
                base: b.clone(),
                quote: c.clone(),
            },
            AssetPair {
                base: c.clone(),
                quote: d.clone(),
            },
            AssetPair {
                base: a.clone(),
                quote: d.clone(),
            },
        ];

        let paths = find_all_paths(&env, &graph, &a, &d, 3);
        assert_eq!(paths.len(), 2); // A->D and A->B->C->D
    }

    #[test]
    fn test_slippage_limit() {
        let env = Env::default();
        let token_a = create_asset(&env, "TOKENA");
        let token_b = create_asset(&env, "TOKENB");
        let pair = AssetPair {
            base: token_a.clone(),
            quote: token_b.clone(),
        };

        set_price(&env, &pair, PRECISION);
        add_available_pair(&env, pair);

        // Huge amount to trigger high slippage (1B units > 10k units triggers volume slippage)
        // 1B units = 10^9. Threshold is 1000.
        // volume_slippage = (10^9 - 1000) / 10000 * 5 bps
        // (1,000,000,000 - 1000) / 10000 * 5 = 100,000 * 5 = 500,000 bps = 5000%
        let result = find_optimal_path(&env, token_a, token_b, 1_000_000_000 * PRECISION);
        assert!(result.is_err()); // Should hit SlippageExceeded
    }

    #[test]
    fn test_path_caching() {
        let env = Env::default();
        let a = create_asset(&env, "A");
        let b = create_asset(&env, "B");
        let pair = AssetPair {
            base: a.clone(),
            quote: b.clone(),
        };

        set_price(&env, &pair, PRECISION);
        add_available_pair(&env, pair.clone());

        let path1 = find_optimal_path(&env, a.clone(), b.clone(), 100 * PRECISION).unwrap();

        // Remove pair from available but it should still be in cache
        let mut pairs = storage::get_available_pairs(&env);
        pairs.remove(pair.clone());
        env.storage()
            .persistent()
            .set(&storage::StorageKey::AvailablePairs, &pairs);

        let path2 = find_optimal_path(&env, a.clone(), b.clone(), 100 * PRECISION).unwrap();
        assert_eq!(path1.hops.len(), path2.hops.len());
    }
}
