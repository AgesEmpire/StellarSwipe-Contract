//! TTL-based result cache for expensive aggregate analytics queries (Issue #753).
//!
//! Designated expensive entrypoints check this cache before recomputing.
//! Cache entries live in Soroban temporary storage (avoids persistent rent costs).
//! TTL is admin-configurable per query type.  Entries past their TTL are treated
//! as misses and trigger a fresh computation with the result replacing the stale entry.
//!
//! Limitation: the cache does not auto-invalidate on underlying state changes
//! within its TTL window.  Callers needing up-to-the-ledger accuracy can call
//! `invalidate_cache` to force a recompute before the TTL expires.

use soroban_sdk::{contracttype, symbol_short, Env};

// ── Types ──────────────────────────────────────────────────────────────────────

/// A cached analytics result with the ledger timestamp at which it was computed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedResult {
    /// Serialised result value (stored as i128 for numeric aggregates).
    pub value: i128,
    /// Ledger timestamp when this result was computed and cached.
    pub cached_at: u64,
}

/// Identifies a cacheable query type.
/// Discriminant values are stable — add new variants with new numbers only.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum QueryType {
    /// Total volume aggregated across all snapshots.
    TotalVolume = 0,
    /// Active-signal count (most recent snapshot).
    ActiveSignals = 1,
    /// Total execution count (most recent snapshot).
    TotalExecutions = 2,
    /// Average provider success rate in basis points (most recent snapshot).
    AvgSuccessRateBps = 3,
}

// ── Storage keys ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
enum CacheKey {
    /// Cached result for a specific query type (temporary storage).
    Entry(u32),
    /// Admin-configured TTL in seconds for a query type (instance storage).
    Ttl(u32),
}

// ── Default TTLs ───────────────────────────────────────────────────────────────

const DEFAULT_TTL_SECS: u64 = 300; // 5 minutes

// ── Public API ─────────────────────────────────────────────────────────────────

/// Admin: configure the TTL (seconds) for a specific query type.
/// Pass 0 to disable caching for that query type (always recompute).
pub fn set_ttl(env: &Env, query_type: QueryType, ttl_secs: u64) {
    env.storage()
        .instance()
        .set(&CacheKey::Ttl(query_type as u32), &ttl_secs);
}

/// Returns the configured TTL for a query type (falls back to DEFAULT_TTL_SECS).
pub fn get_ttl(env: &Env, query_type: QueryType) -> u64 {
    env.storage()
        .instance()
        .get::<_, u64>(&CacheKey::Ttl(query_type as u32))
        .unwrap_or(DEFAULT_TTL_SECS)
}

/// Attempt to read a still-valid cached result for `query_type`.
/// Returns `Some(value)` if the cache is a hit, `None` if it is a miss or expired.
pub fn read(env: &Env, query_type: QueryType) -> Option<i128> {
    let ttl = get_ttl(env, query_type);
    if ttl == 0 {
        return None; // Caching disabled for this query type.
    }

    let entry: Option<CachedResult> = env
        .storage()
        .temporary()
        .get(&CacheKey::Entry(query_type as u32));

    if let Some(cached) = entry {
        let now = env.ledger().timestamp();
        if now.saturating_sub(cached.cached_at) < ttl {
            return Some(cached.value); // Cache hit within TTL.
        }
        // TTL elapsed — fall through as a miss; caller will recompute.
    }
    None
}

/// Store a freshly computed result in the cache.
pub fn write(env: &Env, query_type: QueryType, value: i128) {
    let ttl = get_ttl(env, query_type);
    if ttl == 0 {
        return; // Caching disabled.
    }

    let entry = CachedResult {
        value,
        cached_at: env.ledger().timestamp(),
    };
    env.storage()
        .temporary()
        .set(&CacheKey::Entry(query_type as u32), &entry);

    // Keep the temporary entry alive for the TTL duration (in ledgers ~5 s each).
    let ttl_ledgers = (ttl / 5).max(1) as u32;
    env.storage()
        .temporary()
        .extend_ttl(&CacheKey::Entry(query_type as u32), ttl_ledgers, ttl_ledgers);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("analytics"), symbol_short!("cache_set")),
        (query_type as u32, value, env.ledger().timestamp()),
    );
}

/// Admin or anyone: explicitly invalidate (remove) the cached entry for a query type.
/// The next call to the corresponding query entrypoint will recompute from scratch.
pub fn invalidate(env: &Env, query_type: QueryType) {
    env.storage()
        .temporary()
        .remove(&CacheKey::Entry(query_type as u32));
}
