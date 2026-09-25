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
            return 10000; // Maximum profit factor
        }
        return 0;
    }
    
    let loss_abs = total_loss.abs();
    ((total_profit * 100) / loss_abs).max(0) as u32
}

/// Calculate Sharpe ratio (simplified version * 100)
/// Sharpe = (Average Return - Risk Free Rate) / Standard Deviation
pub fn calculate_sharpe_ratio(
    avg_return: i128,
    std_deviation: i128,
    risk_free_rate: i128,
) -> i32 {
    if std_deviation == 0 {
        return 0;
    }
    
    let excess_return = avg_return - risk_free_rate;
    ((excess_return * 100) / std_deviation) as i32
}

/// Calculate maximum drawdown percentage
pub fn calculate_max_drawdown(peak_value: i128, trough_value: i128) -> u32 {
    if peak_value <= 0 {
        return 0;
    }
    
    let drawdown = peak_value - trough_value;
    if drawdown <= 0 {
        return 0;
    }
    
    ((drawdown * 10000) / peak_value) as u32
}

/// Calculate consistency score (0-100)
/// Based on variance of returns and win rate stability
pub fn calculate_consistency_score(
    win_rate_variance: u32,
    return_variance: i128,
) -> u32 {
    // Lower variance = higher consistency
    // This is a simplified calculation
    let wr_score = 100u32.saturating_sub(win_rate_variance.min(100));
    let rv_score = 100u32.saturating_sub((return_variance.abs() / 1000).min(100) as u32);
    
    (wr_score + rv_score) / 2
}

/// Calculate risk score (0-100)
/// Higher score = higher risk
pub fn calculate_risk_score(
    max_drawdown: u32,
    volatility: u32,
    leverage_used: u32,
) -> u32 {
    let dd_component = (max_drawdown / 100).min(40);
    let vol_component = (volatility / 100).min(40);
    let lev_component = (leverage_used / 10).min(20);
    
    dd_component + vol_component + lev_component
}

// ============================================================================
// Historical Trend Analysis
// ============================================================================

/// Analyze historical performance trends
pub fn analyze_historical_trend(
    data_points: &Vec<TimeSeriesDataPoint>,
) -> TrendDirection {
    if data_points.len() < 2 {
        return TrendDirection::Sideways;
    }
    
    // Calculate simple moving average trend
    let recent_avg = calculate_recent_average(data_points, 5);
    let older_avg = calculate_older_average(data_points, 5);
    
    let diff_percentage = if older_avg != 0 {
        ((recent_avg - older_avg) * 100) / older_avg.abs()
    } else {
        0
    };
    
    match diff_percentage {
        d if d > 20 => TrendDirection::StrongUptrend,
        d if d > 5 => TrendDirection::Uptrend,
        d if d < -20 => TrendDirection::StrongDowntrend,
        d if d < -5 => TrendDirection::Downtrend,
        _ => TrendDirection::Sideways,
    }
}

/// Calculate recent average from data points
fn calculate_recent_average(data_points: &Vec<TimeSeriesDataPoint>, count: usize) -> i128 {
    let len = data_points.len();
    if len == 0 {
        return 0;
    }
    
    let start = if len > count { len - count } else { 0 };
    let mut sum = 0i128;
    let mut actual_count = 0u32;
    
    for i in start..len {
        if let Some(dp) = data_points.get(i as u32) {
            sum += dp.value;
            actual_count += 1;
        }
    }
    
    if actual_count > 0 {
        sum / actual_count as i128
    } else {
        0
    }
}

/// Calculate older average from data points
fn calculate_older_average(data_points: &Vec<TimeSeriesDataPoint>, count: usize) -> i128 {
    let len = data_points.len();
    if len <= count {
        return 0;
    }
    
    let end = len - count;
    let start = if end > count { end - count } else { 0 };
    let mut sum = 0i128;
    let mut actual_count = 0u32;
    
    for i in start..end {
        if let Some(dp) = data_points.get(i as u32) {
            sum += dp.value;
            actual_count += 1;
        }
    }
    
    if actual_count > 0 {
        sum / actual_count as i128
    } else {
        0
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_new_record_is_deterministic() {
        let a = estimate_storage_rent(ContractOperation::CreateRecord, 128, 0, 0);
        let b = estimate_storage_rent(ContractOperation::CreateRecord, 128, 0, 0);
        assert_eq!(a, b);
        assert_eq!(a.projected_entries, 1);
        assert_eq!(a.projected_bytes, 128 + ENTRY_OVERHEAD_BYTES);
        assert_eq!(a.ttl_impact, DEFAULT_TTL_EXTENSION);
        assert!(a.within_limits);
    }

    #[test]
    fn estimate_update_record_reports_ttl_impact() {
        let estimate = estimate_storage_rent(ContractOperation::UpdateRecord, 256, 10, 500);
        assert_eq!(estimate.projected_entries, 1);
        assert_eq!(estimate.projected_bytes, 256 + ENTRY_OVERHEAD_BYTES);
        assert_eq!(estimate.ttl_impact, 500);
        assert_eq!(
            estimate.projected_rent,
            (estimate.projected_bytes as i128) * 500
        );
        assert!(estimate.within_limits);
    }

    #[test]
    fn estimate_near_limit_request_is_flagged() {
        let estimate = estimate_storage_rent(
            ContractOperation::AppendEntry,
            MAX_BYTES_PER_OPERATION,
            MAX_ENTRIES_PER_OPERATION,
            0,
        );
        assert!(!estimate.within_limits);
    }

    #[test]
    fn estimate_remove_entry_frees_bytes() {
        let estimate = estimate_storage_rent(ContractOperation::RemoveEntry, 512, 5, 0);
        assert_eq!(estimate.projected_entries, 1);
        assert_eq!(estimate.projected_bytes, 0);
        assert_eq!(estimate.projected_rent, 0);
        assert!(estimate.within_limits);
    }
}
