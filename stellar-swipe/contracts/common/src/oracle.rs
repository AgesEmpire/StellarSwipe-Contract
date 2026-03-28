#![allow(dead_code)]
//! Oracle interface shared across contracts.
//!
//! `IOracleClient` is the canonical trait for fetching manipulation-resistant
//! prices.  The real implementation calls an on-chain oracle contract via
//! `soroban_sdk::invoke`; the mock implementation is used in unit tests.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

// ── Types ────────────────────────────────────────────────────────────────────

/// A price reading returned by any oracle implementation.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OraclePrice {
    /// Raw price value (scaled by 10^decimals).
    pub price: i128,
    /// Number of decimal places used to scale `price`.
    pub decimals: u32,
    /// Unix timestamp (seconds) when the price was last updated.
    pub timestamp: u64,
    /// Short identifier of the price source (e.g. `Symbol::new(env, "band")`).
    pub source: Symbol,
}

/// Errors that an oracle call can return.
#[contracttype]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum OracleError {
    /// No oracle address has been configured.
    NotConfigured = 1,
    /// The oracle contract returned no price for this asset pair.
    PriceNotFound = 2,
    /// The price is older than the acceptable staleness window.
    PriceStale = 3,
    /// The oracle contract call failed.
    CallFailed = 4,
}

// ── Trait ────────────────────────────────────────────────────────────────────

/// Minimal oracle interface.  Implement this trait to swap between the real
/// on-chain oracle and a test mock without changing any call-site code.
pub trait IOracleClient {
    /// Fetch the latest price for `asset_pair` (e.g. `"XLM/USDC"`).
    fn get_price(&self, env: &Env, asset_pair: u32) -> Result<OraclePrice, OracleError>;
}

// ── On-chain client ───────────────────────────────────────────────────────────

/// Calls the deployed oracle contract via cross-contract invocation.
pub struct OnChainOracleClient {
    pub address: Address,
}

impl IOracleClient for OnChainOracleClient {
    fn get_price(&self, env: &Env, asset_pair: u32) -> Result<OraclePrice, OracleError> {
        // Cross-contract call: oracle_contract.get_price(asset_pair) -> OraclePrice
        let result: Option<OraclePrice> = env
            .invoke_contract(
                &self.address,
                &Symbol::new(env, "get_price"),
                soroban_sdk::vec![env, asset_pair.into()],
            );
        result.ok_or(OracleError::PriceNotFound)
    }
}

// ── Mock client (test-only) ───────────────────────────────────────────────────

/// In-memory mock oracle.  Prices are seeded via `set_price` before tests run.
/// Stored in `Env::storage().temporary()` so each test environment is isolated.
pub struct MockOracleClient;

const MOCK_ORACLE_KEY: &str = "mock_oracle";

impl MockOracleClient {
    /// Seed a price for `asset_pair` in the test environment.
    pub fn set_price(env: &Env, asset_pair: u32, price: OraclePrice) {
        env.storage()
            .temporary()
            .set(&(symbol_short!("mock_orc"), asset_pair), &price);
    }

    /// Remove a previously seeded price (simulates oracle unavailability).
    pub fn clear_price(env: &Env, asset_pair: u32) {
        env.storage()
            .temporary()
            .remove(&(symbol_short!("mock_orc"), asset_pair));
    }
}

impl IOracleClient for MockOracleClient {
    fn get_price(&self, env: &Env, asset_pair: u32) -> Result<OraclePrice, OracleError> {
        env.storage()
            .temporary()
            .get(&(symbol_short!("mock_orc"), asset_pair))
            .ok_or(OracleError::PriceNotFound)
    }
}
