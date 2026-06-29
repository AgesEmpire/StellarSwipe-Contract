//! Per-asset update frequency SLA monitoring (Issue #670).
//!
//! Tracks rolling average update intervals per asset pair and emits
//! an `sla_breach` event when the rolling average exceeds the configured SLA.

use soroban_sdk::{contracttype, symbol_short, Env, Symbol};
use stellar_swipe_common::AssetPair;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Stores the rolling-average update cadence and SLA config for one pair.
#[contracttype]
#[derive(Clone, Debug)]
pub struct FeedSlaRecord {
    /// Admin-configured maximum acceptable average interval (seconds).
    pub expected_interval_secs: u64,
    /// Running sum of observed inter-update intervals (seconds).
    pub interval_sum: u64,
    /// Number of intervals recorded (one less than number of updates).
    pub interval_count: u64,
    /// Timestamp of the previous update (0 = no prior update).
    pub prev_update_ts: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct FeedHealth {
    /// Rolling average seconds between updates (0 if < 2 updates seen).
    pub avg_update_interval_secs: u64,
    /// Configured SLA (seconds). 0 = no SLA configured.
    pub expected_interval_secs: u64,
    /// True when avg_update_interval_secs > expected_interval_secs (and both > 0).
    pub sla_breached: bool,
    /// Total update intervals recorded.
    pub interval_count: u64,
}

#[contracttype]
#[derive(Clone)]
enum SlaKey {
    Record(AssetPair),
}

// ── Storage ───────────────────────────────────────────────────────────────────

fn load(env: &Env, pair: &AssetPair) -> FeedSlaRecord {
    env.storage()
        .persistent()
        .get(&SlaKey::Record(pair.clone()))
        .unwrap_or(FeedSlaRecord {
            expected_interval_secs: 0,
            interval_sum: 0,
            interval_count: 0,
            prev_update_ts: 0,
        })
}

fn save(env: &Env, pair: &AssetPair, record: &FeedSlaRecord) {
    env.storage()
        .persistent()
        .set(&SlaKey::Record(pair.clone()), record);
}

// ── Admin ─────────────────────────────────────────────────────────────────────

/// Set the admin-configurable expected update interval SLA for a pair.
pub fn set_sla(env: &Env, pair: &AssetPair, expected_interval_secs: u64) {
    let mut record = load(env, pair);
    record.expected_interval_secs = expected_interval_secs;
    save(env, pair, &record);
}

// ── Update hook ───────────────────────────────────────────────────────────────

/// Called on each price update. Updates the rolling average and emits
/// `sla_breach` if the new average exceeds the configured SLA.
pub fn record_update(env: &Env, pair: &AssetPair) {
    let now = env.ledger().timestamp();
    let mut record = load(env, pair);

    if record.prev_update_ts > 0 && now > record.prev_update_ts {
        let interval = now - record.prev_update_ts;
        record.interval_sum = record.interval_sum.saturating_add(interval);
        record.interval_count = record.interval_count.saturating_add(1);

        // Check SLA breach
        if record.expected_interval_secs > 0 && record.interval_count > 0 {
            let avg = record.interval_sum / record.interval_count;
            if avg > record.expected_interval_secs {
                env.events().publish(
                    (Symbol::new(env, "sla_breach"),),
                    (pair.clone(), avg, record.expected_interval_secs),
                );
            }
        }
    }

    record.prev_update_ts = now;
    save(env, pair, &record);
}

// ── Query ─────────────────────────────────────────────────────────────────────

/// Read-only summary of recent update cadence for an asset pair.
pub fn get_feed_health(env: &Env, pair: &AssetPair) -> FeedHealth {
    let record = load(env, pair);
    let avg = if record.interval_count > 0 {
        record.interval_sum / record.interval_count
    } else {
        0
    };
    let sla_breached =
        record.expected_interval_secs > 0 && avg > 0 && avg > record.expected_interval_secs;
    FeedHealth {
        avg_update_interval_secs: avg,
        expected_interval_secs: record.expected_interval_secs,
        sla_breached,
        interval_count: record.interval_count,
    }
}
