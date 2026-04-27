#![no_std]

mod admin;
mod conversion;
mod errors;
mod events;
mod external_adapter;
mod history;
mod multi_hop;
mod reputation;
mod sdex;
mod staleness;
mod storage;
mod types;

use errors::OracleError;
use reputation::{
    adjust_oracle_weight, calculate_reputation, get_oracle_stats, should_remove_oracle,
    slash_oracle, track_oracle_accuracy, SlashReason,
};
use sdex::{calculate_spot_price, OrderBook, OrderEntry};
use soroban_sdk::{contract, contractimpl, symbol_short, vec, Address, Env, Map, String, Vec};
use staleness::{OracleHealth, OracleStatus, StalenessLevel};
use stellar_swipe_common::emergency::{PauseState, CAT_ALL};
use stellar_swipe_common::{
    health_uninitialized, placeholder_admin, Asset, AssetPair, HealthStatus,
};
use types::{
    ConsensusPriceData, ExternalPrice, OracleReputation, PriceData, PriceSubmission, StorageKey,
};

pub use conversion::{convert_to_base, ConversionPath};
pub use history::{calculate_twap, get_historical_price, get_twap_deviation, store_price};
pub use multi_hop::{calculate_multi_hop_price, find_optimal_path, LiquidityPath};
pub use storage::{get_base_currency, get_price, set_base_currency, set_price};

#[contract]
pub struct OracleContract;

#[contractimpl]
impl OracleContract {
    /// # Summary
    /// One-time oracle initialization. Sets the admin and base currency.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `admin`: Address that will hold admin privileges.
    /// - `base_currency`: The base asset all prices are quoted against.
    ///
    /// # Returns
    /// Nothing. Panics if already initialized.
    pub fn initialize(env: Env, admin: Address, base_currency: Asset) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        storage::set_base_currency(&env, base_currency);
    }

    /// Read-only health probe for monitoring and front-ends (no auth).
    pub fn health_check(env: Env) -> HealthStatus {
        let version = String::from_str(&env, env!("CARGO_PKG_VERSION"));
        if !env.storage().instance().has(&StorageKey::Admin) {
            return health_uninitialized(&env, version);
        }
        let admin = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .unwrap_or_else(|| placeholder_admin(&env));
        let is_paused = admin::is_paused(&env, String::from_str(&env, CAT_ALL));
        HealthStatus {
            is_initialized: true,
            is_paused,
            version,
            admin,
        }
    }

    /// # Summary
    /// Set price for an asset pair. Stores the price, updates history,
    /// and triggers staleness metadata update.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `pair`: The asset pair to price.
    /// - `price`: Price value (must be > 0).
    ///
    /// # Returns
    /// `Ok(())` on success.
    ///
    /// # Errors
    /// - [`OracleError::CircuitBreakerTripped`] — oracle is paused.
    /// - [`OracleError::InvalidAsset`] — price <= 0.
    pub fn set_price(env: Env, pair: AssetPair, price: i128) -> Result<(), OracleError> {
        if admin::is_paused(&env, String::from_str(&env, CAT_ALL)) {
            return Err(OracleError::CircuitBreakerTripped);
        }
        if price <= 0 {
            return Err(OracleError::InvalidAsset);
        }
        storage::set_price(&env, &pair, price);
        storage::add_available_pair(&env, pair.clone());
        history::store_price(&env, &pair, price);
        on_price_update(&env, pair);
        Ok(())
    }

    /// Convert amount to base currency
    pub fn convert_to_base(env: Env, amount: i128, asset: Asset) -> Result<i128, OracleError> {
        // Check cache first
        let base = storage::get_base_currency(&env);
        if let Some(cached) = storage::get_cached_conversion(&env, &asset, &base) {
            return Ok(amount
                .checked_mul(cached.rate)
                .and_then(|v| v.checked_div(10_000_000))
                .ok_or(OracleError::ConversionOverflow)?);
        }

        // Perform conversion
        let result = conversion::convert_to_base(&env, amount, asset.clone())?;

        // Cache the rate
        if amount > 0 {
            let rate = result
                .checked_mul(10_000_000)
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

    /// Get historical price at timestamp
    pub fn get_historical_price(env: Env, pair: AssetPair, timestamp: u64) -> Option<i128> {
        history::get_historical_price(&env, &pair, timestamp)
    }

    /// Check oracle heartbeat health for a pair using ledger freshness.
    pub fn check_oracle_heartbeat(env: Env, pair: AssetPair) -> OracleHealth {
        let health = staleness::check_oracle_heartbeat(&env, &pair);
        maybe_emit_heartbeat_missed(&env, &pair, &health);
        health
    }

    /// Get current pause states
    pub fn get_pause_states(env: Env) -> Map<String, PauseState> {
        admin::get_pause_states(&env)
    }

    /// Pause a category (admin or guardian)
    pub fn pause_category(
        env: Env,
        caller: Address,
        category: String,
        duration: Option<u64>,
        reason: String,
    ) -> Result<(), OracleError> {
        admin::pause_category(&env, &caller, category, duration, reason)
    }

    /// Unpause a category (admin only)
    pub fn unpause_category(
        env: Env,
        caller: Address,
        category: String,
    ) -> Result<(), OracleError> {
        admin::unpause_category(&env, &caller, category)
    }

    /// Set guardian address (admin only)
    pub fn set_guardian(env: Env, caller: Address, guardian: Address) -> Result<(), OracleError> {
        admin::set_guardian(&env, &caller, guardian)
    }

    /// Revoke guardian (admin only)
    pub fn revoke_guardian(env: Env, caller: Address) -> Result<(), OracleError> {
        admin::revoke_guardian(&env, &caller)
    }

    /// Get current guardian, if any.
    pub fn get_guardian(env: Env) -> Option<Address> {
        admin::get_guardian(&env)
    }

    /// Propose admin transfer (current admin only)
    pub fn propose_admin_transfer(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), OracleError> {
        admin::propose_admin_transfer(&env, &caller, new_admin)
    }

    /// Accept admin transfer (new admin only)
    pub fn accept_admin_transfer(env: Env, caller: Address) -> Result<(), OracleError> {
        admin::accept_admin_transfer(&env, &caller)
    }

    /// Cancel pending admin transfer (current admin only)
    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), OracleError> {
        admin::cancel_admin_transfer(&env, &caller)
    }

    /// Calculate TWAP for 1 hour
    pub fn get_twap_1h(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
        history::calculate_twap(&env, &pair, 3600)
    }

    /// Calculate TWAP for 24 hours
    pub fn get_twap_24h(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
        history::calculate_twap(&env, &pair, 86400)
    }

    /// Calculate TWAP for 7 days
    pub fn get_twap_7d(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
        history::calculate_twap(&env, &pair, 604800)
    }

    /// Get price deviation from TWAP
    pub fn get_price_deviation(
        env: Env,
        pair: AssetPair,
        current_price: i128,
        window: u64,
    ) -> Result<i128, OracleError> {
        history::get_twap_deviation(&env, &pair, current_price, window)
    }

    /// Find optimal path between assets
    pub fn find_optimal_path(
        env: Env,
        from: Asset,
        to: Asset,
        amount: i128,
    ) -> Result<LiquidityPath, OracleError> {
        multi_hop::find_optimal_path(&env, from, to, amount)
    }

    /// Calculate price via multi-hop path
    pub fn calculate_multi_hop_price(env: Env, path: LiquidityPath, amount: i128) -> i128 {
        multi_hop::calculate_multi_hop_price(&env, path, amount)
    }

    /// Register a new oracle
    pub fn register_oracle(env: Env, admin: Address, oracle: Address) -> Result<(), OracleError> {
        admin.require_auth();
        Self::require_admin(&env, &admin)?;

        let mut oracles = Self::read_oracles(&env);
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
        if admin::is_paused(&env, String::from_str(&env, CAT_ALL)) {
            return Err(OracleError::CircuitBreakerTripped);
        }
        oracle.require_auth();

        if price <= 0 {
            return Err(OracleError::InvalidPrice);
        }

        let oracles = Self::read_oracles(&env);
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
        let oracles = Self::read_oracles(&env);

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
            num_oracles: submissions.len() as u32,
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

    fn read_oracles(env: &Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&StorageKey::Oracles)
            .unwrap_or(Vec::new(env))
    }

    /// Get all registered oracles
    pub fn get_oracles(env: Env) -> Vec<Address> {
        Self::read_oracles(&env)
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
        let oracles = Self::read_oracles(env);
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

    /// # Summary
    /// Get the aggregated price for an asset pair. Applies median aggregation
    /// across all fresh price sources (staleness TTL: 300s).
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `pair`: The asset pair to query.
    ///
    /// # Returns
    /// The median aggregated price.
    ///
    /// # Errors
    /// - [`OracleError::PriceNotFound`] — no price data for this pair.
    /// - [`OracleError::StalePrice`] — all price sources are stale (> 300s old).
    /// - [`OracleError::UnreliablePrice`] — sources disagree by > 10%.
    pub fn get_price(env: Env, pair: AssetPair) -> Result<i128, OracleError> {
        let (price, _) = Self::get_price_with_confidence(env, pair)?;
        Ok(price)
    }

    pub fn get_price_with_confidence(
        env: Env,
        pair: AssetPair,
    ) -> Result<(i128, u32), OracleError> {
        let key = StorageKey::PriceMap(pair.clone());
        let prices: Vec<PriceData> = env
            .storage()
            .temporary()
            .get(&key)
            .ok_or(OracleError::PriceNotFound)?;

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

    pub fn add_price_source(
        env: Env,
        admin: Address,
        source: Address,
        weight: u32,
    ) -> Result<(), OracleError> {
        admin.require_auth();
        Self::require_admin(&env, &admin)?;

        env.storage()
            .persistent()
            .set(&StorageKey::OracleWeight(source), &weight);
        Ok(())
    }

    /// Submit a price observation for aggregation (`PriceMap` path).
    pub fn submit_pair_price(
        env: Env,
        source: Address,
        pair: AssetPair,
        price: i128,
        confidence: u32,
    ) -> Result<(), OracleError> {
        if admin::is_paused(&env, String::from_str(&env, CAT_ALL)) {
            return Err(OracleError::CircuitBreakerTripped);
        }

        source.require_auth();

        // Ensure source is a registered oracle
        let weight: u32 = env
            .storage()
            .persistent()
            .get(&StorageKey::OracleWeight(source.clone()))
            .unwrap_or(0);
        if weight == 0 {
            return Err(OracleError::Unauthorized);
        }

        let key = StorageKey::PriceMap(pair.clone());
        let mut prices: Vec<PriceData> = env
            .storage()
            .temporary()
            .get(&key)
            .unwrap_or(Vec::new(&env));

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

        Ok(price)
    }

    pub fn update_with_external_data(
        env: Env,
        prices: Vec<ExternalPrice>,
    ) -> Result<i128, OracleError> {
        let first_pair = prices.get(0).map(|p| p.asset_pair.clone());
        let consensus_price = crate::external_adapter::process_external_prices(&env, prices)?;
        if let Some(pair) = first_pair {
            storage::set_price(&env, &pair, consensus_price);
            on_price_update(&env, pair);
        }

        Ok(consensus_price)
    }
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
        env.events().publish(
            (symbol_short!("RECOVER"), pair.clone()),
            env.ledger().timestamp(),
        );
    }

    metadata.last_update = env.ledger().timestamp();
    metadata.last_update_ledger = env.ledger().sequence();
    metadata.update_count_24h += 1;
    metadata.last_heartbeat_status = OracleStatus::Healthy;
    staleness::set_metadata(env, &pair, metadata);
}

fn maybe_emit_heartbeat_missed(env: &Env, pair: &AssetPair, health: &OracleHealth) {
    if health.status == OracleStatus::Healthy {
        return;
    }

    let mut metadata = staleness::get_metadata(env, pair);
    if metadata.last_heartbeat_status != health.status {
        events::emit_oracle_heartbeat_missed(
            env,
            health.status.clone(),
            health.last_update_ledger,
            health.ledgers_since_update,
        );
        metadata.last_heartbeat_status = health.status.clone();
        staleness::set_metadata(env, pair, metadata);
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod test_health;

#[cfg(test)]
mod test_admin_transfer;
