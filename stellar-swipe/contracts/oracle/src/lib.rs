#![no_std]

 feature/signal-categorization-tagging
 feature/signal-categorization-tagging
=======
 feature/oracle-price-conversion
 main
mod conversion;
mod errors;
mod storage;

use soroban_sdk::{contract, contractimpl, Address, Env};
use common::{Asset, AssetPair};
use errors::OracleError;

pub use conversion::{convert_to_base, ConversionPath};
pub use storage::{get_base_currency, set_base_currency, get_price, set_price};
=======
mod errors;
mod events;
mod reputation;
mod types;

use errors::OracleError;
use reputation::{
    adjust_oracle_weight, calculate_reputation, get_oracle_stats, should_remove_oracle,
    slash_oracle, track_oracle_accuracy, SlashReason,
};
use soroban_sdk::{contract, contractimpl, Address, Env, Vec};
 feature/signal-categorization-tagging
use types::{ConsensusPriceData, OracleReputation, PriceSubmission, StorageKey};
 main
=======
use types::{ConsensusPriceData, OracleReputation, PriceSubmission, StorageKey}; main
 main

#[contract]
pub struct OracleContract;

#[contractimpl]
impl OracleContract {
 feature/signal-categorization-tagging
 feature/signal-categorization-tagging
=======
 feature/oracle-price-conversion
 main
    /// Initialize oracle with base currency
    pub fn initialize(env: Env, admin: Address, base_currency: Asset) {
        storage::set_base_currency(&env, base_currency);
    }

    /// Set price for an asset pair
    pub fn set_price(env: Env, pair: AssetPair, price: i128) -> Result<(), OracleError> {
        if price <= 0 {
            return Err(OracleError::InvalidAsset);
        }
        storage::set_price(&env, &pair, price);
        storage::add_available_pair(&env, pair);
        Ok(())
    }

    /// Get price for an asset pair
    pub fn get_price(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
        storage::get_price(&env, &pair)
    }

    /// Convert amount to base currency
    pub fn convert_to_base(env: Env, amount: i128, asset: Asset) -> Result<i128, OracleError> {
        // Check cache first
        let base = storage::get_base_currency(&env);
        if let Some(cached) = storage::get_cached_conversion(&env, &asset, &base) {
            return Ok(amount.checked_mul(cached.rate)
                .and_then(|v| v.checked_div(10_000_000))
                .ok_or(OracleError::ConversionOverflow)?);
        }

        // Perform conversion
        let result = conversion::convert_to_base(&env, amount, asset.clone())?;
        
        // Cache the rate
        if amount > 0 {
            let rate = result.checked_mul(10_000_000)
                .and_then(|v| v.checked_div(amount))
                .unwrap_or(0);
            if rate > 0 {
                storage::set_cached_conversion(&env, &asset, &base, rate);
            }
        }
        
        Ok(result)
    }

    /// Get base currency
    pub fn get_base_currency(env: Env) -> Asset {
        storage::get_base_currency(&env)
    }

    /// Set base currency (admin only)
    pub fn set_base_currency(env: Env, asset: Asset) {
        storage::set_base_currency(&env, asset);
    }

    /// Add available trading pair
    pub fn add_pair(env: Env, pair: AssetPair) {
        storage::add_available_pair(&env, pair);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, String};

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

    fn token(env: &Env, code: &str) -> Asset {
        Asset {
            code: String::from_str(env, code),
            issuer: Some(Address::generate(env)),
        }
    }

    #[test]
    fn test_initialize_and_get_base() {
        let env = Env::default();
        let contract_id = env.register_contract(None, OracleContract);
        let client = OracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);

        client.initialize(&admin, &xlm(&env));
        let base = client.get_base_currency();
        assert_eq!(base.code, String::from_str(&env, "XLM"));
    }

    #[test]
    fn test_set_and_get_price() {
        let env = Env::default();
        let contract_id = env.register_contract(None, OracleContract);
        let client = OracleContractClient::new(&env, &contract_id);

        let pair = AssetPair {
            base: usdc(&env),
            quote: xlm(&env),
        };
        
        client.set_price(&pair, &10_000_000); // 1 USDC = 1 XLM
        let price = client.get_price(&pair).unwrap();
        assert_eq!(price, 10_000_000);
    }

    #[test]
    fn test_convert_to_base_direct() {
        let env = Env::default();
        let contract_id = env.register_contract(None, OracleContract);
        let client = OracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);

        let xlm = xlm(&env);
        let usdc = usdc(&env);

        client.initialize(&admin, &xlm);

        // Set price: 1 USDC = 10 XLM
        let pair = AssetPair {
            base: usdc.clone(),
            quote: xlm.clone(),
        };
        client.set_price(&pair, &100_000_000); // 10 XLM per USDC

        // Convert 100 USDC to XLM
        let result = client.convert_to_base(&100_0000000, &usdc).unwrap();
        assert_eq!(result, 1000_0000000); // 1000 XLM
    }

    #[test]
    fn test_convert_same_asset() {
        let env = Env::default();
        let contract_id = env.register_contract(None, OracleContract);
        let client = OracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);

        let xlm = xlm(&env);
        client.initialize(&admin, &xlm);

        let result = client.convert_to_base(&1000_0000000, &xlm).unwrap();
        assert_eq!(result, 1000_0000000);
    }

    #[test]
    fn test_base_currency_change() {
        let env = Env::default();
        let contract_id = env.register_contract(None, OracleContract);
        let client = OracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);

        let xlm = xlm(&env);
        let usdc = usdc(&env);

        client.initialize(&admin, &xlm);
        assert_eq!(client.get_base_currency().code, String::from_str(&env, "XLM"));

        client.set_base_currency(&usdc);
        assert_eq!(client.get_base_currency().code, String::from_str(&env, "USDC"));
    }
}
=======
    /// Initialize the contract with an admin
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    /// Register a new oracle
    pub fn register_oracle(env: Env, admin: Address, oracle: Address) -> Result<(), OracleError> {
        admin.require_auth();
        Self::require_admin(&env, &admin)?;

        let mut oracles = Self::get_oracles(&env);
        if oracles.contains(&oracle) {
            return Err(OracleError::OracleAlreadyExists);
        }

        oracles.push_back(oracle.clone());
        env.storage()
            .persistent()
            .set(&StorageKey::Oracles, &oracles);

        // Initialize with default reputation
        let stats = OracleReputation {
            total_submissions: 0,
            accurate_submissions: 0,
            avg_deviation: 0,
            reputation_score: 50,
            weight: 1,
            last_slash: 0,
        };
        reputation::save_oracle_stats(&env, &oracle, &stats);

        Ok(())
    }

    /// Submit a price from an oracle
    pub fn submit_price(env: Env, oracle: Address, price: i128) -> Result<(), OracleError> {
        oracle.require_auth();

        if price <= 0 {
            return Err(OracleError::InvalidPrice);
        }

        let oracles = Self::get_oracles(&env);
        if !oracles.contains(&oracle) {
            return Err(OracleError::OracleNotFound);
        }

        // Check reputation
        let stats = get_oracle_stats(&env, &oracle);
        if stats.weight == 0 {
            return Err(OracleError::LowReputation);
        }

        let submission = PriceSubmission {
            oracle: oracle.clone(),
            price,
            timestamp: env.ledger().timestamp(),
        };

        let mut submissions = Self::get_price_submissions(&env);
        submissions.push_back(submission);
        env.storage()
            .instance()
            .set(&StorageKey::PriceSubmissions, &submissions);

        events::emit_price_submitted(&env, oracle, price);

        Ok(())
    }

    /// Calculate consensus price and update oracle reputations
    pub fn calculate_consensus(env: Env) -> Result<i128, OracleError> {
        let submissions = Self::get_price_submissions(&env);
        let oracles = Self::get_oracles(&env);

        if submissions.is_empty() {
            return Err(OracleError::InsufficientOracles);
        }

        // Calculate weighted median
        let consensus_price = Self::weighted_median(&env, &submissions);

        // Track accuracy for each oracle
        for i in 0..submissions.len() {
            let submission = submissions.get(i).unwrap();
            track_oracle_accuracy(&env, &submission.oracle, submission.price, consensus_price);

            // Check for major deviation and slash if needed
            let deviation = ((submission.price - consensus_price).abs() * 10000) / consensus_price;
            if deviation > 2000 {
                // 20%
                slash_oracle(&env, &submission.oracle, SlashReason::MajorDeviation);
                events::emit_oracle_slashed(&env, submission.oracle.clone(), "major_deviation", 20);
            }
        }

        // Adjust weights for all oracles
        let mut removed_oracles = Vec::new(&env);
        for i in 0..oracles.len() {
            let oracle = oracles.get(i).unwrap();
            let old_stats = get_oracle_stats(&env, &oracle);
            let old_weight = old_stats.weight;

            let new_weight = adjust_oracle_weight(&env, &oracle);

            if new_weight != old_weight {
                let reputation = calculate_reputation(&env, &oracle);
                events::emit_weight_adjusted(
                    &env,
                    oracle.clone(),
                    old_weight,
                    new_weight,
                    reputation,
                );
            }

            if should_remove_oracle(&env, &oracle) {
                removed_oracles.push_back(oracle.clone());
            }
        }

        // Remove poor performing oracles (but keep minimum 2)
        if oracles.len() - removed_oracles.len() >= 2 {
            for i in 0..removed_oracles.len() {
                let oracle = removed_oracles.get(i).unwrap();
                Self::remove_oracle_internal(&env, &oracle);
                events::emit_oracle_removed(&env, oracle, "Low reputation");
            }
        }

        // Store consensus
        let consensus_data = ConsensusPriceData {
            price: consensus_price,
            timestamp: env.ledger().timestamp(),
            num_oracles: submissions.len(),
        };
        env.storage()
            .persistent()
            .set(&StorageKey::ConsensusPrice, &consensus_data);

        // Clear submissions for next round
        env.storage().instance().set(
            &StorageKey::PriceSubmissions,
            &Vec::<PriceSubmission>::new(&env),
        );

        events::emit_consensus_reached(&env, consensus_price, submissions.len());

        Ok(consensus_price)
    }

    /// Get oracle reputation stats
    pub fn get_oracle_reputation(env: Env, oracle: Address) -> OracleReputation {
        get_oracle_stats(&env, &oracle)
    }

    /// Get all registered oracles
    pub fn get_oracles(env: &Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&StorageKey::Oracles)
            .unwrap_or(Vec::new(env))
    }

    /// Get current consensus price
    pub fn get_consensus_price(env: Env) -> Option<ConsensusPriceData> {
        env.storage().persistent().get(&StorageKey::ConsensusPrice)
    }

    /// Remove an oracle (admin only)
    pub fn remove_oracle(env: Env, admin: Address, oracle: Address) -> Result<(), OracleError> {
        admin.require_auth();
        Self::require_admin(&env, &admin)?;
        Self::remove_oracle_internal(&env, &oracle);
        Ok(())
    }

    // Internal helpers

    fn require_admin(env: &Env, caller: &Address) -> Result<(), OracleError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(OracleError::Unauthorized)?;

        if caller != &admin {
            return Err(OracleError::Unauthorized);
        }
        Ok(())
    }

    fn get_price_submissions(env: &Env) -> Vec<PriceSubmission> {
        env.storage()
            .instance()
            .get(&StorageKey::PriceSubmissions)
            .unwrap_or(Vec::new(env))
    }

    fn weighted_median(env: &Env, submissions: &Vec<PriceSubmission>) -> i128 {
        if submissions.is_empty() {
            return 0;
        }

        // Create weighted list
        let mut weighted_prices = Vec::new(env);
        for i in 0..submissions.len() {
            let submission = submissions.get(i).unwrap();
            let stats = get_oracle_stats(env, &submission.oracle);
            let weight = stats.weight.max(1);

            for _ in 0..weight {
                weighted_prices.push_back(submission.price);
            }
        }

        // Sort prices
        let len = weighted_prices.len();
        for i in 0..len {
            for j in 0..(len - i - 1) {
                let curr = weighted_prices.get(j).unwrap();
                let next = weighted_prices.get(j + 1).unwrap();
                if curr > next {
                    weighted_prices.set(j, next);
                    weighted_prices.set(j + 1, curr);
                }
            }
        }

        // Return median
        let mid = len / 2;
        if len % 2 == 0 {
            (weighted_prices.get(mid - 1).unwrap() + weighted_prices.get(mid).unwrap()) / 2
        } else {
            weighted_prices.get(mid).unwrap()
        }
    }

    fn remove_oracle_internal(env: &Env, oracle: &Address) {
        let oracles = Self::get_oracles(env);
        let mut new_oracles = Vec::new(env);

        for i in 0..oracles.len() {
            let o = oracles.get(i).unwrap();
            if o != *oracle {
                new_oracles.push_back(o);
            }
        }

        env.storage()
            .persistent()
            .set(&StorageKey::Oracles, &new_oracles);
    }

    /// returns aggregated price
    pub fn get_price(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
        let (price, _) = Self::get_price_with_confidence(env, pair)?;
        Ok(price)
    }

    pub fn get_price_with_confidence(env: Env, pair: AssetPair) -> Result<(i128, u32), OracleError> {
        let key = StorageKey::PriceMap(pair.clone());
        let prices: Vec<PriceData> = env.storage().temporary().get(&key).ok_or(OracleError::PriceNotFound)?;

        let current_time = env.ledger().timestamp();
        let mut fresh_prices: Vec<PriceData> = Vec::new(&env);

        // 1. Filter stale prices (TTL: 300s / 5 mins)
        for p in prices.iter() {
            if current_time.saturating_sub(p.timestamp) < 300 {
                fresh_prices.push_back(p);
            }
        }

        if fresh_prices.is_empty() {
            return Err(OracleError::StalePrice);
        }

        // 2. Median Aggregation
        // Sort by price
        let mut sorted = fresh_prices;
        let len = sorted.len();
        for i in 0..len {
            for j in 0..(len - i - 1) {
                if sorted.get(j).unwrap().price > sorted.get(j + 1).unwrap().price {
                    let temp = sorted.get(j).unwrap();
                    sorted.set(j, sorted.get(j + 1).unwrap());
                    sorted.set(j + 1, temp);
                }
            }
        }

        let median_data = sorted.get(len / 2).unwrap();
        
        // 3. Check for 10% deviation (Edge Case)
        let min_p = sorted.get(0).unwrap().price;
        let max_p = sorted.get(len - 1).unwrap().price;
        if (max_p - min_p) * 100 / min_p > 10 {
            // Price sources disagree by > 10%
            return Err(OracleError::UnreliablePrice); 
        }

        Ok((median_data.price, median_data.confidence))
    }

    pub fn add_price_source(env: Env, admin: Address, source: Address, weight: u32) -> Result<(), OracleError> {
        admin.require_auth();
        // Check if caller is admin (logic from your existing lib.rs)
        Self::is_admin(&env, &admin)?;
        
        env.storage().persistent().set(&StorageKey::OracleWeight(source), &weight);
        Ok(())
    }

    pub fn submit_price(env: Env, source: Address, pair: AssetPair, price: i128, confidence: u32) -> Result<(), OracleError> {
        source.require_auth();
        
        // Ensure source is a registered oracle
        let weight: u32 = env.storage().persistent().get(&StorageKey::OracleWeight(source.clone())).unwrap_or(0);
        if weight == 0 { return Err(OracleError::Unauthorized); }

        let key = StorageKey::PriceMap(pair.clone());
        let mut prices: Vec<PriceData> = env.storage().temporary().get(&key).unwrap_or(Vec::new(&env));

        let new_entry = PriceData {
            asset_pair: pair,
            price,
            timestamp: env.ledger().timestamp(),
            source,
            confidence,
        };

        prices.push_back(new_entry);
        
        // Cache management: Keep prices for 5 mins
        env.storage().temporary().set(&key, &prices);
        env.storage().temporary().extend_ttl(&key, 60, 60); 
        
        Ok(())
    }

    pub fn refresh_from_sdex(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
        // 1. In a real Soroban scenario, you would interface with the 
        // Liquidity Pool or a specialized SDEX oracle contract.
        // For this issue, we assume we fetch the orderbook.
        let orderbook = fetch_sdex_orderbook(&env, &pair)?;

        // 2. Calculate price
        let price = calculate_spot_price(&env, orderbook)?;

        // 3. Store in the PriceMap (from Issue #32)
        let source_address = env.current_contract_address();
        Self::submit_internal(&env, source_address, pair, price, 100)?;

        Ok(price)
    }

    pub fn update_with_external_data(env: Env, prices: Vec<ExternalPrice>) -> Result<i128, OracleError> {
        // Process the external prices using our new adapter
        let consensus_price = crate::external_adapter::process_external_prices(&env, prices)?;

        // Store the result using your existing storage logic
        // (Assuming the first price in the vec defines the pair for this update)
        if let Some(first) = prices.get(0) {
            storage::set_price(&env, &first.asset_pair, consensus_price);
        }

        Ok(consensus_price)
    }

// Internal helper to represent the SDEX query
fn fetch_sdex_orderbook(env: &Env, pair: &AssetPair) -> Result<OrderBook, OracleError> {
    // Note: Actual Soroban host functions for SDEX are currently limited 
    // to Liquidity Pool swaps. For Order Books, one typically uses 
    // a Cross-Chain/Bridge approach or a Trusted Observer.
    // Here we implement the interface logic.
    unimplemented!("SDEX Orderbook Host Interface");
}

pub fn get_safe_price(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
    let level = staleness::check_staleness(&env, pair.clone());
    
    if level == StalenessLevel::Critical {
        return Err(OracleError::CircuitBreakerTripped);
    }
    
    if level == StalenessLevel::Stale {
        return Err(OracleError::PriceStaleTradeBlocked);
    }

    storage::get_price(&env, &pair)
}

pub fn on_price_update(env: &Env, pair: AssetPair) {
    let mut metadata = staleness::get_metadata(env, &pair);
    
    // Auto-recovery
    if metadata.is_paused {
        metadata.is_paused = false;
        env.events().publish((symbol_short!("RECOVER"), pair.clone()), env.ledger().timestamp());
    }
    
    metadata.last_update = env.ledger().timestamp();
    metadata.update_count_24h += 1;
    staleness::set_metadata(env, &pair, metadata);
}

#[cfg(test)]
mod test;
 main
