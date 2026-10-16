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

/* … truncated 6237 chars — edit only what you need near the top … */
