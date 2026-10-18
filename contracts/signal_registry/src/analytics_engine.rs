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
// Analytics Query Cost Envelopes
// ============================================================================

/// The kind of analytics query a caller intends to run. Each variant maps to a
/// declared cost envelope so callers can reject expensive requests before
/// execution.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum AnalyticsQueryType {
    /// Aggregate a single provider's headline metrics.
    ProviderSummary,
    /// Build a time-series over a bounded window.
    TimeSeries,
    /// Compute period performance over a bounded window.
    PeriodPerformance,
    /// Produce predictive analytics for a provider.
    Predictive,
    /// Scan for anomalies across a provider's history.
    AnomalyScan,
    /// Assemble a full performance report.
    FullReport,
}

/// Bounded cost metadata attached to an analytics query. `max_scan_cost` and
/// `max_aggregation_cost` are the declared ceilings for the query type; a
/// request whose estimated costs exceed either ceiling must be rejected before
/// execution.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct QueryCostEnvelope {
    pub query_type: AnalyticsQueryType,
    pub max_scan_cost: u32,
    pub max_aggregation_cost: u32,
}

/// Which cost dimension a request exceeded.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum CostDimension {
    Scan,
    Aggregation,
}

/// Error returned when a query's estimated cost exceeds its declared envelope.
/// `dimension` identifies whether the scan or aggregation budget was exceeded.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct CostEnvelopeExceeded {
    pub query_type: AnalyticsQueryType,
    pub dimension: CostDimension,
    pub limit: u32,
    pub estimated: u32,
}

/// Declared cost envelope for a query type. Every query type must declare a
/// maximum scan cost and a maximum aggregation cost.
pub fn query_cost_envelope(query_type: AnalyticsQueryType) -> QueryCostEnvelope {
    let (max_scan_cost, max_aggregation_cost) = match query_type {
        AnalyticsQueryType::ProviderSummary => (100u32, 50u32),
        AnalyticsQueryType::TimeSeries => (1_000u32, 500u32),
        AnalyticsQueryType::PeriodPerformance => (2_000u32, 1_000u32),
        AnalyticsQueryType::Predictive => (500u32, 2_000u32),
        AnalyticsQueryType::AnomalyScan => (5_000u32, 2_500u32),
        AnalyticsQueryType::FullReport => (10_000u32, 5_000u32),
    };

    QueryCostEnvelope {
        query_type,
        max_scan_cost,
        max_aggregation_cost,
    }
}

/// Enforce a query's declared cost envelope before execution.
///
/// `estimated_scan_cost` and `estimated_aggregation_cost` are the caller's
/// projected costs for the request. Returns `Ok(())` when both dimensions are
/// within the declared envelope, or `Err(CostEnvelopeExceeded)` identifying the
/// first exceeded dimension (scan checked before aggregation).
pub fn enforce_query_cost_envelope(
    query_type: AnalyticsQueryType,
    estimated_scan_cost: u32,
    estimated_aggregation_cost: u32,
) -> Result<(), CostEnvelopeExceeded> {
    let envelope = query_cost_envelope(query_type);

    if estimated_scan_cost > envelope.max_scan_cost {
        return Err(CostEnvelopeExceeded {
            query_type,
            dimension: CostDimension::Scan,
            limit: envelope.max_scan_cost,
            estimated: estimated_scan_cost,
        });
    }

    if estimated_aggregation_cost > envelope.max_aggregation_cost {
        return Err(CostEnvelopeExceeded {
            query_type,
            dimension: CostDimension::Aggregation,
            limit: envelope.max_aggregation_cost,
            estimated: estimated_aggregation_cost,
        });
    }

    Ok(())
}

// ============================================================================
// Risk Score Feature Versioning
// ============================================================================

/// Version of the feature set / formula used to compute a provider risk score.
///
/// Risk scores are only interpretable relative to the feature version that
/// produced them. When the underlying formula changes, a new version is
/// introduced and historical scores keep referencing the version they were
/// computed with.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum RiskScoreFeatureVersion {
    /// Initial risk score formula.
    V1,
    /// Revised risk score formula.
    V2,
}

/// The feature version currently used for newly computed risk scores.
const CURRENT_RISK_SCORE_FEATURE_VERSION: RiskScoreFeatureVersion =
    RiskScoreFeatureVersion::V2;

/// A provider risk score together with the feature version that produced it.
///
/// Recording the version alongside the score keeps historical scores
/// interpretable after a formula update.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VersionedRiskScore {
    pub provider: Address,
    pub score: u32,                 // 0-100
    pub feature_version: RiskScoreFeatureVersion,
    pub computed_at: u64,
}

/// Typed error returned when a risk score feature version is not supported.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum RiskScoreVersionError {
    /// The requested feature version is not supported by this contract.
    UnsupportedFeatureVersion { requested: RiskScoreFeatureVersion },
}

/// Return the feature version used for newly computed risk scores.
pub fn current_risk_score_feature_version() -> RiskScoreFeatureVersion {
    CURRENT_RISK_SCORE_FEATURE_VERSION
}

/// Check whether a feature version is supported for risk score computation.
pub fn is_supported_risk_score_feature_version(
    version: &RiskScoreFeatureVersion,
) -> bool {
    matches!(
        version,
        RiskScoreFeatureVersion::V1 | RiskScoreFeatureVersion::V2
    )
}

/// Compute a provider risk score using an explicit feature version.
///
/// Unsupported feature versions fail explicitly with a typed
/// [`RiskScoreVersionError`] rather than silently falling back to the current
/// formula, so callers can never misinterpret a score.
///
pub fn compute_versioned_risk_score(
    env: &Env,
    provider: Address,
    score: u32,
    feature_version: RiskScoreFeatureVersion,
) -> Result<VersionedRiskScore, RiskScoreVersionError> {
    if !is_supported_risk_score_feature_version(&feature_version) {
        return Err(RiskScoreVersionError::UnsupportedFeatureVersion {
            requested: feature_version,
        });
    }
    Ok(VersionedRiskScore {
        provider,
        score,
        feature_version,
        computed_at: env.ledger().timestamp(),
    })
}

/// Compute a provider risk score using the current feature version.
pub fn compute_current_risk_score(
    env: &Env,
    provider: Address,
    score: u32,
) -> VersionedRiskScore {
    VersionedRiskScore {
        provider,
        score,
        feature_version: CURRENT_RISK_SCORE_FEATURE_VERSION,
        computed_at: env.ledger().timestamp(),
    }
}

/// Feature version metadata exposed through the analytics query API.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RiskScoreVersionMetadata {
    /// Feature version applied to newly computed risk scores.
    pub current_version: RiskScoreFeatureVersion,
    /// All feature versions this contract can interpret.
    pub supported_versions: Vec<RiskScoreFeatureVersion>,
}

/// Expose risk score feature version metadata for the analytics query API.
pub fn risk_score_version_metadata(env: &Env) -> RiskScoreVersionMetadata {
    let mut supported_versions = Vec::new(env);
    supported_versions.push_back(RiskScoreFeatureVersion::V1);
    supported_versions.push_back(RiskScoreFeatureVersion::V2);
    RiskScoreVersionMetadata {
        current_version: CURRENT_RISK_SCORE_FEATURE_VERSION,
        supported_versions,
    }
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
    /// Projected byte footprint of the affec

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
            return u32::MAX;
        }
        return 0;
    }
    ((total_profit as u64 * 100) / total_loss as u64) as u32
}
