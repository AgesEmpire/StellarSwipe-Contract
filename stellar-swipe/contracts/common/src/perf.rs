//! Performance utilities for high-frequency trading paths.
//!
//! Provides transaction-scoped caching (temporary storage), operation profiling
//! hooks, and baseline constants for regression tests.

use soroban_sdk::{contracttype, symbol_short, Env, Symbol};

/// Soroban default per-transaction CPU instruction budget.
pub const DEFAULT_INSTRUCTION_BUDGET: u64 = 100_000_000;

/// Regression threshold: no single hot-path op should exceed 80% of budget.
pub const REGRESSION_BUDGET_PCT: u64 = 80;

/// Recommended max instructions for a single copy-trade execution.
pub const BASELINE_COPY_TRADE_INSTRUCTIONS: u64 = 8_000_000;

/// Recommended max instructions for a single auto_trade execution.
pub const BASELINE_AUTO_TRADE_INSTRUCTIONS: u64 = 12_000_000;

/// Recommended max instructions for fee collection (without oracle deferral).
pub const BASELINE_FEE_COLLECT_INSTRUCTIONS: u64 = 5_000_000;

/// Recommended max instructions for signal submission at moderate scale.
pub const BASELINE_SIGNAL_SUBMIT_INSTRUCTIONS: u64 = 15_000_000;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PerfStorageKey {
    /// Generic tx-scoped cache slot (auto-evicted after tx).
    TxCache(Symbol),
    /// Cumulative profile marker written by `mark_operation`.
    OpProfile(Symbol),
}

/// Read a value from the transaction-scoped cache, or compute and store it.
///
/// Note: despite the "transaction-scoped" naming, this is backed by Soroban
/// **temporary** storage, which is keyed on `(contract, key)` and survives
/// until its TTL lapses — it is *not* wiped at the end of a single
/// transaction. A cached value can therefore be served to many subsequent
/// transactions. Callers that cache values derived from mutable contract
/// config MUST call [`invalidate_tx_cache`] for the same `key` whenever that
/// underlying config changes, or stale values will be served until the TTL
/// expires (see issue #801).
pub fn tx_cache_or_compute<T, F>(env: &Env, key: Symbol, mut compute: F) -> T
where
    T: Clone
        + soroban_sdk::IntoVal<Env, soroban_sdk::Val>
        + soroban_sdk::TryFromVal<Env, soroban_sdk::Val>,
    F: FnMut() -> T,
{
    let cache_key = PerfStorageKey::TxCache(key);
    if let Some(cached) = env.storage().temporary().get::<_, T>(&cache_key) {
        return cached;
    }
    let value = compute();
    env.storage().temporary().set(&cache_key, &value);
    value
}

/// Evict a transaction-scoped cache entry so the next [`tx_cache_or_compute`]
/// call for the same `key` recomputes from source-of-truth storage instead of
/// returning a stale cached value.
///
/// Call this immediately after writing any config value that feeds a cached
/// computation (e.g. a fee rate config field cached by `fee_collector`), so
/// the change takes effect on the very next read rather than waiting for the
/// temporary storage entry's TTL to lapse.
pub fn invalidate_tx_cache(env: &Env, key: Symbol) {
    let cache_key = PerfStorageKey::TxCache(key);
    env.storage().temporary().remove(&cache_key);
}

/// Record that an operation completed (stores timestamp for off-chain latency analysis).
pub fn mark_operation(env: &Env, op: Symbol) {
    let key = PerfStorageKey::OpProfile(op);
    env.storage()
        .temporary()
        .set(&key, &env.ledger().timestamp());
}

/// Returns the instruction budget threshold used in regression tests.
pub fn regression_budget_limit() -> u64 {
    DEFAULT_INSTRUCTION_BUDGET * REGRESSION_BUDGET_PCT / 100
}

/// Symbol keys for standard hot-path operations (used in profiling events).
pub fn op_execute_trade() -> Symbol {
    symbol_short!("exec_trd")
}

pub fn op_collect_fee() -> Symbol {
    symbol_short!("collect")
}

pub fn op_batch_execute() -> Symbol {
    symbol_short!("batch")
}

pub fn op_create_signal() -> Symbol {
    symbol_short!("sig_sub")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regression_budget_limit() {
        assert_eq!(regression_budget_limit(), 80_000_000);
    }
}
