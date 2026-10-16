// Advanced Analytics Engine for Signal Performance
// Provides deep insights, historical analysis, and predictions

use soroban_sdk::{contracttype, Address, Env, Vec};

// ============================================================================
// Data Models
// ============================================================================

/// Core analytics data model for signal providers
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct SignalProviderAnalytics {
    pub provider: Address,
    pub total_signals: u32,
    pub successful_signals: u32,
    pub failed_signals: u32,
    pub total_profit: i128,
    pub total_loss: i128,
    pub avg_profit_per_signal: i128,
    pub win_rate: u32,              // Percentage (0-10000 for 0.00% - 100.00%)
    pub profit_factor: u32,         // Ratio * 100
    pub sharpe_ratio: i32,          // Ratio * 100 (can be negative)
    pub max_drawdown: u32,          // Percentage
    pub avg_holding_period: u64,    // Seconds
    pub consistency_score: u32,     // 0-100
    pub risk_score: u32,            // 0-100
    pub last_updated: u64,
}

/// Time-series data point for historical analysis
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct TimeSeriesDataPoint {
    pub timestamp: u64,
    pub value: i128,
    pub signal_count: u32,
    pub win_rate: u32,
}

/// Performance metrics over a specific period
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PeriodPerformance {
    pub period_start: u64,
    pub period_end: u64,
    pub total_signals: u32,
    pub win_rate: u32,
    pub total_pnl: i128,
    pub avg_pnl: i128,
    pub volatility: u32,
    pub best_signal_pnl: i128,
    pub worst_signal_pnl: i128,
}

/// Predictive analytics result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PredictiveAnalytics {
    pub provider: Address,
    pub predicted_win_rate: u32,
    pub confidence_level: u32,      // 0-100
    pub trend_direction: TrendDirection,
    pub risk_level: RiskLevel,
    pub recommendation: Recommendation,
}

/// Trend direction enum
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum TrendDirection {
    StrongUptrend,
    Uptrend,
    Sideways,
    Downtrend,
    StrongDowntrend,
}

/// Risk level classification
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum RiskLevel {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}

/// Recommendation for signal provider
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum Recommendation {
    StrongBuy,
    Buy,
    Hold,
    Sell,
    StrongSell,
}

/// Anomaly detection result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct AnomalyDetection {
    pub provider: Address,
    pub anomaly_type: AnomalyType,
    pub severity: u32,              // 0-100
    pub detected_at: u64,
    pub description: String,
}

/// Types of anomalies that can be detected
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum AnomalyType {
    SuddenPerformanceDrop,
    UnusuallyHighWinRate,
    SuspiciousPattern,
    VolatilitySpike,
    DrawdownExceeded,
    InactivityPeriod,
}

/// Performance report structure
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PerformanceReport {
    pub provider: Address,
    pub report_period: PeriodPerformance,
    pub analytics: SignalProviderAnalytics,
    pub historical_trend: Vec<TimeSeriesDataPoint>,
    pub predictions: PredictiveAnalytics,
    pub anomalies: Vec<AnomalyDetection>,
    pub generated_at: u64,
}

// ============================================================================
// Analytics Snapshot Pagination Cursors
// ============================================================================

/// Identity of the analytics snapshot a pagination cursor was issued against.
///
/// A snapshot is identified by the ledger sequence at which it was produced
/// together with a monotonically increasing version. Any change to the
/// underlying analytics data bumps the version, which invalidates every cursor
/// that was issued against an earlier snapshot.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct AnalyticsSnapshotId {
    /// Ledger sequence at which the snapshot was produced.
    pub ledger: u32,
    /// Monotonic version of the snapshot; bumped on every data change.
    pub version: u64,
}

/// Pagination cursor for analytics snapshots.
///
/// The cursor carries the identity of the snapshot it was issued against and
/// the ledger sequence at which it expires. Clients must present the cursor
/// unchanged on the next page request; combining pages from different
/// snapshots is rejected.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct AnalyticsCursor {
    /// Snapshot the cursor was issued against.
    pub snapshot: AnalyticsSnapshotId,
    /// Offset of the next page within the snapshot.
    pub offset: u32,
    /// Ledger sequence at which the cursor expires (exclusive).
    pub expires_at: u32,
}

/// Typed error returned when a pagination cursor can no longer be used.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum AnalyticsCursorError {
    /// The cursor was issued against a snapshot that is no longer current.
    SnapshotChanged {
        expected: AnalyticsSnapshotId,
        current: AnalyticsSnapshotId,
    },
    /// The cursor has passed its expiry ledger.
    Expired { expires_at: u32, current_ledger: u32 },
}

/// Default number of ledgers a cursor remains valid for.
const CURSOR_TTL_LEDGERS: u32 = 100;

/// Issue a fresh pagination cursor against the latest snapshot.
///
/// New cursors always start from the latest snapshot identity and carry an
/// expiry derived from the current ledger sequence.
///
pub fn issue_analytics_cursor(
    env: &Env,
    snapshot: AnalyticsSnapshotId,
    offset: u32,
) -> AnalyticsCursor {
    AnalyticsCursor {
        snapshot,
        offset,
        expires_at: env.ledger().sequence().saturating_add(CURSOR_TTL_LEDGERS),
    }
}

/// Validate a pagination cursor against the current snapshot and ledger.
///
/// Returns `Ok(())` when the cursor still refers to the current snapshot and
/// has not expired. Otherwise returns a typed [`AnalyticsCursorError`] so
/// callers can distinguish a stale snapshot from an expired cursor.
///
pub fn validate_analytics_cursor(
    env: &Env,
    cursor: &AnalyticsCursor,
    current: &AnalyticsSnapshotId,
) -> Result<(), AnalyticsCursorError> {
    let current_ledger = env.ledger().sequence();
    if current_ledger >= cursor.expires_at {
        return Err(AnalyticsCursorError::Expired {
            expires_at: cursor.expires_at,
            current_ledger,
        });
    }
    if cursor.snapshot != *current {
        return Err(AnalyticsCursorError::SnapshotChanged {
            expected: cursor.snapshot.clone(),
            current: current.clone(),
        });
    }
    Ok(())
}

// ============================================================================
// Storage Rent Estimation Diagnostics
// ============================================================================

/// Common contract operations whose storage footprint can be estimated
/// before submission.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum ContractOperation {
    /// Persist a brand new record (entry does not yet exist).
    CreateRecord,
    /// Overwrite an existing record with a same-sized payload.
    UpdateRecord,
    /// Append a new entry to an existing collection.
    AppendEntry,
    /// Remove an existing entry from a collection.
    RemoveEntry,
}

/// Projected storage footprint and rent implications for a contract
/// operation. All fields are derived deterministically from the inputs so
/// identical requests always yield identical estimates.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct StorageRentEstimate {
    /// Number of ledger entries the operation is projected to touch.
    pub projected_entries: u32,
    /// Projected byte footprint of the affected entries.
    pub projected_bytes: u32,
    /// Projected change in TTL (in ledgers) for the affected entries.
    pub ttl_impact: u32,
    /// Projected rent cost in stroops for the affected entries.
    pub projected_rent: i128,
    /// Whether the request is within the configured storage limits.
    pub within_limits: bool,
}

/// Baseline byte cost of a single ledger entry (key + metadata overhead).
const ENTRY_OVERHEAD_BYTES: u32 = 64;
/// Rent charged per byte per ledger (in stroops).
const RENT_PER_BYTE_PER_LEDGER: i128 = 1;
/// Default TTL extension applied to touched entries, in ledgers.
const DEFAULT_TTL_EXTENSION: u32 = 100;
/// Maximum number of entries a single operation may touch.
const MAX_ENTRIES_PER_OPERATION: u32 = 1000;
/// Maximum byte footprint a single operation may touch.
const MAX_BYTES_PER_OPERATION: u32 = 1_000_000;

/// Estimate the storage footprint and rent implications of a contract
/// operation before it is submitted.
///
/// The estimate is a pure function of its inputs: the same `operation`,
/// `payload_bytes`, `existing_entries`, and `ttl_extension` always produce the
/// same `StorageRentEstimate`.
///
/// * `payload_bytes` is the serialized size of the record being written.
/// * `existing_entries` is the number of entries already stored for the
///   collection the operation targets.
/// * `ttl_extension` is the number of ledgers the caller intends to extend the
///   touched entries by; pass `0` to use the default extension.
pub fn estimate_storage_rent(
    operation: ContractOperation,
    payload_bytes: u32,
    existing_entries: u32,
    ttl_extension: u32,
) -> StorageRentEstimate {
    let entry_bytes = payload_bytes.saturating_add(ENTRY_OVERHEAD_BYTES);

    let (projected_entries, projected_bytes) = match operation {
        ContractOperation::CreateRecord => (1u32, entry_bytes),
        ContractOperation::UpdateRecord => (1u32, entry_bytes),
        ContractOperation::AppendEntry => (1u32, entry_bytes),
        ContractOperation::RemoveEntry => {
            // Removal touches the entry but frees its bytes.
            (1u32, 0u32)
        }
    };

    let ttl_impact = if ttl_extension == 0 {
        DEFAULT_TTL_EXTENSION
    } else {
        ttl_extension
    };

    let projected_rent = (projected_bytes as i128)
        .saturating_mul(RENT_PER_BYTE_PER_LEDGER)
        .saturating_mul(ttl_impact as i128);

    let total_entries = existing_entries.saturating_add(projected_entries);
    let within_limits = total_entries <= MAX_ENTRIES_PER_OPERATION
        && projected_bytes <= MAX_BYTES_PER_OPERATION;

    StorageRentEstimate {
        projected_entries,
        projected_bytes,
        ttl_impact,
        projected_rent,
        within_limits,
    }
}

// ============================================================================
// Performance Metrics Calculation
// ============================================================================

/// Calculate win rate percentage (0-10000 for 0.00% - 100.00%)
pub fn calculate_win_rate(successful: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    ((successful as u64 * 10000) / total as u64) as u32
}

/// Calculate profit factor (total profit / total loss * 100)
pub fn calculate_profit_factor(total_profit: i128, total_loss: i128) -> u32 {
    if total_loss == 0 {
        if total_profit > 0 {
            return 10000;
        }
        return 0;
    }
    ((total_profit as u64 * 100) / total_loss as u64) as u32
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;

    fn snapshot(env: &Env, version: u64) -> AnalyticsSnapshotId {
        AnalyticsSnapshotId {
            ledger: env.ledger().sequence(),
            version,
        }
    }

    #[test]
    fn new_cursor_targets_latest_snapshot() {
        let env = Env::default();
        let current = snapshot(&env, 7);
        let cursor = issue_analytics_cursor(&env, current.clone(), 0);
        assert_eq!(cursor.snapshot, current);
        assert_eq!(cursor.offset, 0);
        assert!(cursor.expires_at > env.ledger().sequence());
    }

    #[test]
    fn cursor_valid_within_same_snapshot() {
        let env = Env::default();
        let current = snapshot(&env, 1);
        let cursor = issue_analytics_cursor(&env, current.clone(), 10);
        assert_eq!(validate_analytics_cursor(&env, &cursor, &current), Ok(()));
    }

    #[test]
    fn stale_cursor_rejected_after_snapshot_update() {
        let env = Env::default();
        let first = snapshot(&env, 1);
        let cursor = issue_analytics_cursor(&env, first.clone(), 10);

        // Snapshot changes between page requests.
        let updated = snapshot(&env, 2);
        let err = validate_analytics_cursor(&env, &cursor, &updated).unwrap_err();
        assert_eq!(
            err,
            AnalyticsCursorError::SnapshotChanged {
                expected: first,
                current: updated,
            }
        );
    }

    #[test]
    fn expired_cursor_rejected() {
        let env = Env::default();
        let current = snapshot(&env, 1);
        let cursor = issue_analytics_cursor(&env, current.clone(), 0);

        env.ledger().set_sequence_number(cursor.expires_at);
        let err = validate_analytics_cursor(&env, &cursor, &current).unwrap_err();
        assert_eq!(
            err,
            AnalyticsCursorError::Expired {
                expires_at: cursor.expires_at,
                current_ledger: cursor.expires_at,
            }
        );
    }

    #[test]
    fn new_cursor_after_update_starts_from_latest_snapshot() {
        let env = Env::default();
        let first = snapshot(&env, 1);
        let stale = issue_analytics_cursor(&env, first, 10);

        let updated = snapshot(&env, 2);
        let fresh = issue_analytics_cursor(&env, updated.clone(), 0);

        assert_eq!(fresh.snapshot, updated);
        assert_eq!(validate_analytics_cursor(&env, &fresh, &updated), Ok(()));
        assert!(validate_analytics_cursor(&env, &stale, &updated).is_err());
    }
}
