//! Per-source heartbeat windows for oracle feeds (Issue #1406).
//!
//! A heartbeat is the maximum interval (seconds) a source may go without
//! reporting before its latest observation for a feed is considered expired.
//! It is configured per feed (a default for every source of that feed) and
//! may be overridden per `(feed, source)`.
//!
//! # Resolution
//! 1. `(pair, source)` override, if set;
//! 2. otherwise the feed-wide heartbeat for `pair`, if set;
//! 3. otherwise no heartbeat applies (only the staleness window does).
//!
//! # Expiry boundary
//! An observation recorded at `observed_at` is live while
//! `now - observed_at < heartbeat`. At exactly `observed_at + heartbeat` it
//! has expired, matching the half-open staleness window used elsewhere in
//! the oracle. Expired observations are excluded from aggregation and from
//! cross-source deviation checks, and an expired external report is rejected
//! before it can move the accepted price or advance the feed's round.

use soroban_sdk::{contracttype, Address, Env};
use stellar_swipe_common::AssetPair;

use crate::errors::OracleError;
use crate::storage::DAY_IN_LEDGERS;

#[contracttype]
#[derive(Clone)]
enum HeartbeatKey {
    Feed(AssetPair),
    Source(AssetPair, Address),
}

fn write(env: &Env, key: &HeartbeatKey, heartbeat_secs: u64) {
    if heartbeat_secs == 0 {
        env.storage().persistent().remove(key);
        return;
    }
    env.storage().persistent().set(key, &heartbeat_secs);
    env.storage()
        .persistent()
        .extend_ttl(key, DAY_IN_LEDGERS, DAY_IN_LEDGERS);
}

/// Set the feed-wide heartbeat for `pair`. `0` clears it.
pub fn set_feed_heartbeat(env: &Env, pair: &AssetPair, heartbeat_secs: u64) {
    write(env, &HeartbeatKey::Feed(pair.clone()), heartbeat_secs);
}

/// Set a per-source heartbeat override for `pair`. `0` clears it.
pub fn set_source_heartbeat(env: &Env, pair: &AssetPair, source: &Address, heartbeat_secs: u64) {
    write(
        env,
        &HeartbeatKey::Source(pair.clone(), source.clone()),
        heartbeat_secs,
    );
}

/// Feed-wide heartbeat for `pair`, if configured.
pub fn get_feed_heartbeat(env: &Env, pair: &AssetPair) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&HeartbeatKey::Feed(pair.clone()))
}

/// Effective heartbeat for `source` on `pair` (override, then feed default).
pub fn get_heartbeat(env: &Env, pair: &AssetPair, source: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&HeartbeatKey::Source(pair.clone(), source.clone()))
        .or_else(|| get_feed_heartbeat(env, pair))
}

/// Pure boundary rule: live while `now - observed_at < heartbeat_secs`.
pub fn is_live(heartbeat_secs: u64, now: u64, observed_at: u64) -> bool {
    now.saturating_sub(observed_at) < heartbeat_secs
}

/// Whether the observation from `source` recorded at `observed_at` is still
/// inside its heartbeat window. Sources without a heartbeat are always live.
pub fn is_within_heartbeat(env: &Env, pair: &AssetPair, source: &Address, observed_at: u64) -> bool {
    match get_heartbeat(env, pair, source) {
        Some(hb) => is_live(hb, env.ledger().timestamp(), observed_at),
        None => true,
    }
}

/// Reject an observation whose source exceeded its heartbeat.
pub fn ensure_within_heartbeat(
    env: &Env,
    pair: &AssetPair,
    source: &Address,
    observed_at: u64,
) -> Result<(), OracleError> {
    if is_within_heartbeat(env, pair, source, observed_at) {
        Ok(())
    } else {
        Err(OracleError::HeartbeatExpired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_is_half_open() {
        // observed at 1_000 with a 100s heartbeat
        assert!(is_live(100, 1_099, 1_000)); // just before expiry
        assert!(!is_live(100, 1_100, 1_000)); // at expiry
        assert!(!is_live(100, 1_101, 1_000)); // just after expiry
    }

    #[test]
    fn zero_heartbeat_is_never_live() {
        assert!(!is_live(0, 1_000, 1_000));
    }
}
