#![allow(dead_code)]
//! Oracle integration for AutoTradeContract.
//!
//! Provides:
//! - Admin-configurable oracle address
//! - `get_aggregated_price()` — fetches a fresh price or returns `OracleUnavailable`
//! - Oracle circuit breaker — auto-pauses trading when oracle is unavailable,
//!   auto-resets when oracle recovers, admin can manually override

use soroban_sdk::{contracttype, Address, Env, String, Symbol};
use stellar_swipe_common::oracle::{
    IOracleClient, MockOracleClient, OnChainOracleClient, OracleError, OraclePrice,
};

use crate::admin::{AdminStorageKey, require_admin};
use crate::errors::AutoTradeError;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum age (seconds) before a price is considered stale.
pub const MAX_PRICE_AGE_SECS: u64 = 300; // 5 minutes

// ── Circuit breaker state ─────────────────────────────────────────────────────

/// Persisted state of the oracle circuit breaker.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleCircuitBreakerState {
    /// True when the circuit breaker has tripped (oracle unavailable).
    pub triggered: bool,
    /// When the breaker was last tripped (ledger timestamp).
    pub triggered_at: u64,
    /// Admin has manually overridden the breaker — trading allowed even without oracle.
    pub admin_override: bool,
}

impl OracleCircuitBreakerState {
    fn default(env: &Env) -> Self {
        OracleCircuitBreakerState {
            triggered: false,
            triggered_at: env.ledger().timestamp(),
            admin_override: false,
        }
    }
}

// ── Storage helpers ───────────────────────────────────────────────────────────

pub fn get_cb_state(env: &Env) -> OracleCircuitBreakerState {
    env.storage()
        .instance()
        .get(&AdminStorageKey::OracleCircuitBreaker)
        .unwrap_or_else(|| OracleCircuitBreakerState::default(env))
}

fn set_cb_state(env: &Env, state: &OracleCircuitBreakerState) {
    env.storage()
        .instance()
        .set(&AdminStorageKey::OracleCircuitBreaker, state);
}

// ── Admin helpers ─────────────────────────────────────────────────────────────

/// Store the oracle contract address (admin-only).
pub fn set_oracle_address(
    env: &Env,
    caller: &Address,
    oracle: Address,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();
    env.storage()
        .instance()
        .set(&AdminStorageKey::OracleAddress, &oracle);
    env.events().publish(
        (Symbol::new(env, "oracle_set"), caller.clone()),
        oracle,
    );
    Ok(())
}

/// Retrieve the configured oracle address, if any.
pub fn get_oracle_address(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::OracleAddress)
}

/// Admin override: allow trading even when oracle circuit breaker is tripped.
/// Emits `OracleCBOverride` event.
pub fn override_oracle_circuit_breaker(
    env: &Env,
    caller: &Address,
    enabled: bool,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();
    let mut state = get_cb_state(env);
    state.admin_override = enabled;
    set_cb_state(env, &state);
    env.events().publish(
        (Symbol::new(env, "oracle_cb_override"), caller.clone()),
        enabled,
    );
    Ok(())
}

// ── Price fetching ────────────────────────────────────────────────────────────

/// Fetch a price from the configured on-chain oracle.
///
/// Returns `OracleError::NotConfigured` when no oracle address has been set,
/// and `OracleError::PriceStale` when the returned timestamp is too old.
pub fn get_oracle_price(env: &Env, asset_pair: u32) -> Result<OraclePrice, OracleError> {
    let address = get_oracle_address(env).ok_or(OracleError::NotConfigured)?;
    let client = OnChainOracleClient { address };
    let price = client.get_price(env, asset_pair)?;
    validate_freshness(env, &price)?;
    Ok(price)
}

/// Fetch a price using the mock oracle (test environments only).
pub fn get_mock_oracle_price(env: &Env, asset_pair: u32) -> Result<OraclePrice, OracleError> {
    let client = MockOracleClient;
    let price = client.get_price(env, asset_pair)?;
    validate_freshness(env, &price)?;
    Ok(price)
}

/// Aggregated price fetch: tries the configured oracle, updates circuit breaker
/// state based on the result, and returns the price or `OracleUnavailable`.
///
/// - On success: resets any previously tripped circuit breaker (auto-recovery).
/// - On failure: trips the circuit breaker and emits `OracleCircuitBreakerTriggered`.
pub fn get_aggregated_price(
    env: &Env,
    asset_pair: u32,
) -> Result<OraclePrice, AutoTradeError> {
    match get_oracle_price(env, asset_pair) {
        Ok(price) => {
            // Oracle is healthy — auto-reset the circuit breaker if it was tripped
            let mut state = get_cb_state(env);
            if state.triggered {
                state.triggered = false;
                set_cb_state(env, &state);
                env.events().publish(
                    (Symbol::new(env, "oracle_cb_reset"),),
                    asset_pair,
                );
            }
            Ok(price)
        }
        Err(err) => {
            // Oracle unavailable — trip the circuit breaker
            let mut state = get_cb_state(env);
            if !state.triggered {
                state.triggered = true;
                state.triggered_at = env.ledger().timestamp();
                set_cb_state(env, &state);
            }
            let reason = match err {
                OracleError::NotConfigured => String::from_str(env, "oracle_not_configured"),
                OracleError::PriceNotFound => String::from_str(env, "price_not_found"),
                OracleError::PriceStale    => String::from_str(env, "price_stale"),
                OracleError::CallFailed    => String::from_str(env, "call_failed"),
            };
            env.events().publish(
                (Symbol::new(env, "oracle_cb_triggered"),),
                reason,
            );
            Err(AutoTradeError::OracleUnavailable)
        }
    }
}

/// Check the oracle circuit breaker before executing a trade.
///
/// Returns `Ok(())` when trading is allowed, `Err(OracleUnavailable)` when
/// the circuit breaker is tripped and no admin override is active.
///
/// Also attempts auto-recovery: if the oracle is now healthy, the breaker is
/// reset and trading proceeds.
pub fn check_oracle_circuit_breaker(
    env: &Env,
    asset_pair: u32,
) -> Result<(), AutoTradeError> {
    let state = get_cb_state(env);

    // Admin override bypasses the circuit breaker entirely
    if state.admin_override {
        return Ok(());
    }

    // If not triggered, nothing to do
    if !state.triggered {
        return Ok(());
    }

    // Breaker is tripped — attempt auto-recovery by probing the oracle
    match get_oracle_price(env, asset_pair) {
        Ok(_) => {
            // Oracle recovered — reset breaker
            let mut new_state = state;
            new_state.triggered = false;
            set_cb_state(env, &new_state);
            env.events().publish(
                (Symbol::new(env, "oracle_cb_reset"),),
                asset_pair,
            );
            Ok(())
        }
        Err(_) => Err(AutoTradeError::OracleUnavailable),
    }
}

/// Return the oracle price scaled to a plain i128 (same unit as SDEX prices).
///
/// Divides by 10^decimals so callers don't need to know the oracle's scale.
pub fn oracle_price_to_i128(op: &OraclePrice) -> i128 {
    let scale = 10i128.pow(op.decimals);
    if scale == 0 {
        op.price
    } else {
        op.price / scale
    }
}

// ── Internal ──────────────────────────────────────────────────────────────────

fn validate_freshness(env: &Env, price: &OraclePrice) -> Result<(), OracleError> {
    let now = env.ledger().timestamp();
    if price.timestamp == 0 || now.saturating_sub(price.timestamp) > MAX_PRICE_AGE_SECS {
        return Err(OracleError::PriceStale);
    }
    Ok(())
}

// ── Oracle whitelist ──────────────────────────────────────────────────────────

/// Read the whitelist for `asset_pair` from instance storage.
pub fn get_oracle_whitelist(env: &Env, asset_pair: u32) -> soroban_sdk::Vec<Address> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::OracleWhitelist(asset_pair))
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

fn set_oracle_whitelist(env: &Env, asset_pair: u32, list: &soroban_sdk::Vec<Address>) {
    env.storage()
        .instance()
        .set(&AdminStorageKey::OracleWhitelist(asset_pair), list);
}

/// Add `oracle_addr` to the whitelist for `asset_pair` (admin-only).
/// Emits `OracleAdded { asset_pair, oracle }` event.
/// Idempotent — adding an already-present address is a no-op.
pub fn add_oracle(
    env: &Env,
    caller: &Address,
    asset_pair: u32,
    oracle_addr: Address,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();

    let mut list = get_oracle_whitelist(env, asset_pair);
    // Idempotency: skip if already present
    for i in 0..list.len() {
        if list.get(i).unwrap() == oracle_addr {
            return Ok(());
        }
    }
    list.push_back(oracle_addr.clone());
    set_oracle_whitelist(env, asset_pair, &list);

    env.events().publish(
        (Symbol::new(env, "oracle_added"), asset_pair),
        oracle_addr,
    );
    Ok(())
}

/// Remove `oracle_addr` from the whitelist for `asset_pair` (admin-only).
/// Emits `OracleRemoved { asset_pair, oracle }` event.
/// Returns `LastOracleForPair` if removing would leave the pair with no oracle.
pub fn remove_oracle(
    env: &Env,
    caller: &Address,
    asset_pair: u32,
    oracle_addr: Address,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    caller.require_auth();

    let list = get_oracle_whitelist(env, asset_pair);

    // Guard: cannot remove the last oracle for a pair
    if list.len() <= 1 {
        return Err(AutoTradeError::LastOracleForPair);
    }

    let mut new_list = soroban_sdk::Vec::new(env);
    for i in 0..list.len() {
        let entry = list.get(i).unwrap();
        if entry != oracle_addr {
            new_list.push_back(entry);
        }
    }
    set_oracle_whitelist(env, asset_pair, &new_list);

    env.events().publish(
        (Symbol::new(env, "oracle_removed"), asset_pair),
        oracle_addr,
    );
    Ok(())
}

/// Verify that `caller` is in the whitelist for `asset_pair`.
/// Returns `Unauthorized` if not whitelisted.
pub fn require_whitelisted_oracle(
    env: &Env,
    caller: &Address,
    asset_pair: u32,
) -> Result<(), AutoTradeError> {
    let list = get_oracle_whitelist(env, asset_pair);
    for i in 0..list.len() {
        if &list.get(i).unwrap() == caller {
            return Ok(());
        }
    }
    Err(AutoTradeError::Unauthorized)
}

/// Whitelisted oracle pushes a price update for `asset_pair`.
///
/// - Verifies `caller` is in the whitelist for `asset_pair`.
/// - Validates freshness of the supplied price.
/// - Stores the price via `risk::set_asset_price` and `risk::record_price`.
/// - Emits `OraclePriceUpdated { asset_pair, price }` event.
pub fn push_price_update(
    env: &Env,
    caller: &Address,
    asset_pair: u32,
    price: OraclePrice,
) -> Result<(), AutoTradeError> {
    caller.require_auth();
    require_whitelisted_oracle(env, caller, asset_pair)?;
    validate_freshness(env, &price).map_err(|_| AutoTradeError::OracleUnavailable)?;

    let scaled = oracle_price_to_i128(&price);
    crate::risk::set_asset_price(env, asset_pair, scaled);
    crate::risk::record_price(env, asset_pair, scaled);

    env.events().publish(
        (Symbol::new(env, "oracle_price_upd"), asset_pair),
        scaled,
    );
    Ok(())
}
