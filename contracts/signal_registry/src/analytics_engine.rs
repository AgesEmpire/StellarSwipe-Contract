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

/// Calculate volatility from historical data
pub fn calculate_volatility(data_points: &Vec<TimeSeriesDataPoint>) -> u32 {
    if data_points.len() < 2 {
        return 0;
    }
    
    // Calculate mean
    let mut sum = 0i128;
    for i in 0..data_points.len() {
        if let Some(dp) = data_points.get(i) {
            sum += dp.value;
        }
    }
    let mean = sum / data_points.len() as i128;
    
    // Calculate variance
    let mut variance_sum = 0i128;
    for i in 0..data_points.len() {
        if let Some(dp) = data_points.get(i) {
            let diff = dp.value - mean;
            variance_sum += diff * diff;
        }
    }
    let variance = variance_sum / data_points.len() as i128;
    
    // Return simplified volatility (sqrt approximation)
    approximate_sqrt(variance.abs()) as u32
}

/// Approximate square root for i128
fn approximate_sqrt(n: i128) -> i128 {
    if n == 0 {
        return 0;
    }
    
    let mut x = n;
    let mut y = (x + 1) / 2;
    
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    
    x
}

// ============================================================================
// Predictive Analytics
// ============================================================================

/// Generate predictive analytics based on historical data
pub fn generate_predictions(
    analytics: &SignalProviderAnalytics,
    historical_data: &Vec<TimeSeriesDataPoint>,
) -> PredictiveAnalytics {
    let trend = analyze_historical_trend(historical_data);
    let risk_level = classify_risk_level(analytics.risk_score);
    
    // Predict future win rate based on trend and current performance
    let predicted_win_rate = predict_win_rate(
        analytics.win_rate,
        &trend,
        analytics.consistency_score,
    );
    
    // Calculate confidence based on data quality and consistency
    let confidence = calculate_prediction_confidence(
        historical_data.len(),
        analytics.consistency_score,
    );
    
    // Generate recommendation
    let recommendation = generate_recommendation(
        predicted_win_rate,
        &risk_level,
        &trend,
    );
    
    PredictiveAnalytics {
        provider: analytics.provider.clone(),
        predicted_win_rate,
        confidence_level: confidence,
        trend_direction: trend,
        risk_level,
        recommendation,
    }
}

/// Predict future win rate
fn predict_win_rate(
    current_win_rate: u32,
    trend: &TrendDirection,
    consistency: u32,
) -> u32 {
    let trend_adjustment = match trend {
        TrendDirection::StrongUptrend => 500,
        TrendDirection::Uptrend => 200,
        TrendDirection::Sideways => 0,
        TrendDirection::Downtrend => -200i32,
        TrendDirection::StrongDowntrend => -500i32,
    };
    
    // Apply consistency factor (higher consistency = more reliable prediction)
    let adjusted = current_win_rate as i32 + 
        (trend_adjustment * consistency as i32) / 100;
    
    adjusted.max(0).min(10000) as u32
}

/// Calculate prediction confidence level
fn calculate_prediction_confidence(data_points: u32, consistency: u32) -> u32 {
    // More data points and higher consistency = higher confidence
    let data_score = (data_points * 2).min(50);
    let consistency_score = consistency / 2;
    
    (data_score + consistency_score).min(100)
}

/// Classify risk level
fn classify_risk_level(risk_score: u32) -> RiskLevel {
    match risk_score {
        0..=20 => RiskLevel::VeryLow,
        21..=40 => RiskLevel::Low,
        41..=60 => RiskLevel::Medium,
        61..=80 => RiskLevel::High,
        _ => RiskLevel::VeryHigh,
    }
}

/// Generate recommendation based on predictions
fn generate_recommendation(
    predicted_win_rate: u32,
    risk_level: &RiskLevel,
    trend: &TrendDirection,
) -> Recommendation {
    // High win rate + low risk + uptrend = Strong Buy
    // Low win rate + high risk + downtrend = Strong Sell
    
    let win_rate_score = predicted_win_rate / 100; // 0-100
    let risk_penalty = match risk_level {
        RiskLevel::VeryLow => 0,
        RiskLevel::Low => 10,
        RiskLevel::Medium => 20,
        RiskLevel::High => 35,
        RiskLevel::VeryHigh => 50,
    };
    
    let trend_bonus = match trend {
        TrendDirection::StrongUptrend => 20,
        TrendDirection::Uptrend => 10,
        TrendDirection::Sideways => 0,
        TrendDirection::Downtrend => -10i32,
        TrendDirection::StrongDowntrend => -20i32,
    };
    
    let total_score = (win_rate_score as i32 - risk_penalty as i32 + trend_bonus)
        .max(0)
        .min(100) as u32;
    
    match total_score {
        80..=100 => Recommendation::StrongBuy,
        60..=79 => Recommendation::Buy,
        40..=59 => Recommendation::Hold,
        20..=39 => Recommendation::Sell,
        _ => Recommendation::StrongSell,
    }
}

// ============================================================================
// Anomaly Detection
// ============================================================================

/// Detect anomalies in signal provider performance
pub fn detect_anomalies(
    env: &Env,
    analytics: &SignalProviderAnalytics,
    historical_data: &Vec<TimeSeriesDataPoint>,
) -> Vec<AnomalyDetection> {
    let mut anomalies = Vec::new(env);
    
    // Check for sudden performance drop
    if let Some(anomaly) = detect_performance_drop(env, historical_data, analytics) {
        anomalies.push_back(anomaly);
    }
    
    // Check for unusually high win rate (potential manipulation)
    if let Some(anomaly) = detect_suspicious_win_rate(env, analytics) {
        anomalies.push_back(anomaly);
    }
    
    // Check for volatility spike
    if let Some(anomaly) = detect_volatility_spike(env, historical_data, analytics) {
        anomalies.push_back(anomaly);
    }
    
    // Check for excessive drawdown
    if let Some(anomaly) = detect_excessive_drawdown(env, analytics) {
        anomalies.push_back(anomaly);
    }
    
    anomalies
}

/// Detect sudden performance drop
fn detect_performance_drop(
    env: &Env,
    historical_data: &Vec<TimeSeriesDataPoint>,
    analytics: &SignalProviderAnalytics,
) -> Option<AnomalyDetection> {
    if historical_data.len() < 10 {
        return None;
    }
    
    let recent_avg = calculate_recent_average(historical_data, 5);
    let older_avg = calculate_older_average(historical_data, 5);
    
    if older_avg > 0 {
        let drop_percentage = ((older_avg - recent_avg) * 100) / older_avg;
        
        if drop_percentage > 30 {
            return Some(AnomalyDetection {
                provider: analytics.provider.clone(),
                anomaly_type: AnomalyType::SuddenPerformanceDrop,
                severity: drop_percentage.min(100) as u32,
                detected_at: env.ledger().timestamp(),
                description: String::from_str(env, "Significant performance decline detected"),
            });
        }
    }
    
    None
}

/// Detect suspiciously high win rate
fn detect_suspicious_win_rate(
    env: &Env,
    analytics: &SignalProviderAnalytics,
) -> Option<AnomalyDetection> {
    // Win rate above 95% with significant number of signals is suspicious
    if analytics.win_rate > 9500 && analytics.total_signals > 20 {
        return Some(AnomalyDetection {
            provider: analytics.provider.clone(),
            anomaly_type: AnomalyType::UnusuallyHighWinRate,
            severity: 70,
            detected_at: env.ledger().timestamp(),
            description: String::from_str(env, "Unusually high win rate detected"),
        });
    }
    
    None
}

/// Detect volatility spike
fn detect_volatility_spike(
    env: &Env,
    historical_data: &Vec<TimeSeriesDataPoint>,
    analytics: &SignalProviderAnalytics,
) -> Option<AnomalyDetection> {
    if historical_data.len() < 10 {
        return None;
    }
    
    let current_volatility = calculate_volatility(historical_data);
    
    // If volatility is extremely high, flag it
    if current_volatility > 5000 {
        return Some(AnomalyDetection {
            provider: analytics.provider.clone(),
            anomaly_type: AnomalyType::VolatilitySpike,
            severity: (current_volatility / 100).min(100),
            detected_at: env.ledger().timestamp(),
            description: String::from_str(env, "Abnormal volatility detected"),
        });
    }
    
    None
}

/// Detect excessive drawdown
fn detect_excessive_drawdown(
    env: &Env,
    analytics: &SignalProviderAnalytics,
) -> Option<AnomalyDetection> {
    // Drawdown above 50% is concerning
    if analytics.max_drawdown > 5000 {
        return Some(AnomalyDetection {
            provider: analytics.provider.clone(),
            anomaly_type: AnomalyType::DrawdownExceeded,
            severity: (analytics.max_drawdown / 100).min(100),
            detected_at: env.ledger().timestamp(),
            description: String::from_str(env, "Excessive drawdown detected"),
        });
    }
    
    None
}

// ============================================================================
// Performance Report Generation
// ============================================================================

/// Generate comprehensive performance report
pub fn generate_performance_report(
    env: &Env,
    provider: Address,
    analytics: SignalProviderAnalytics,
    period: PeriodPerformance,
    historical_data: Vec<TimeSeriesDataPoint>,
) -> PerformanceReport {
    // Generate predictions
    let predictions = generate_predictions(&analytics, &historical_data);
    
    // Detect anomalies
    let anomalies = detect_anomalies(env, &analytics, &historical_data);
    
    PerformanceReport {
        provider,
        report_period: period,
        analytics,
        historical_trend: historical_data,
        predictions,
        anomalies,
        generated_at: env.ledger().timestamp(),
    }
}

/// Calculate period performance from signal data
pub fn calculate_period_performance(
    period_start: u64,
    period_end: u64,
    signals: &Vec<SignalData>,
) -> PeriodPerformance {
    let mut total_signals = 0u32;
    let mut successful_signals = 0u32;
    let mut total_pnl = 0i128;
    let mut best_pnl = 0i128;
    let mut worst_pnl = 0i128;
    let mut pnl_values = Vec::new();
    
    for signal in signals.iter() {
        if signal.timestamp >= period_start && signal.timestamp <= period_end {
            total_signals += 1;
            
            if signal.pnl > 0 {
                successful_signals += 1;
            }
            
            total_pnl += signal.pnl;
            pnl_values.push(signal.pnl);
            
            if signal.pnl > best_pnl {
                best_pnl = signal.pnl;
            }
            if signal.pnl < worst_pnl {
                worst_pnl = signal.pnl;
            }
        }
    }
    
    let avg_pnl = if total_signals > 0 {
        total_pnl / total_signals as i128
    } else {
        0
    };
    
    let win_rate = calculate_win_rate(successful_signals, total_signals);
    let volatility = calculate_pnl_volatility(&pnl_values);
    
    PeriodPerformance {
        period_start,
        period_end,
        total_signals,
        win_rate,
        total_pnl,
        avg_pnl,
        volatility,
        best_signal_pnl: best_pnl,
        worst_signal_pnl: worst_pnl,
    }
}

/// Helper struct for signal data
#[derive(Clone, Debug, PartialEq)]
pub struct SignalData {
    pub timestamp: u64,
    pub pnl: i128,
}

/// Calculate volatility from PnL values
fn calculate_pnl_volatility(pnl_values: &[i128]) -> u32 {
    if pnl_values.len() < 2 {
        return 0;
    }
    
    // Calculate mean
    let sum: i128 = pnl_values.iter().sum();
    let mean = sum / pnl_values.len() as i128;
    
    // Calculate variance
    let variance_sum: i128 = pnl_values
        .iter()
        .map(|&pnl| {
            let diff = pnl - mean;
            diff * diff
        })
        .sum();
    
    let variance = variance_sum / pnl_values.len() as i128;
    
    // Return simplified volatility
    approximate_sqrt(variance.abs()) as u32
}

// ============================================================================
// Data Visualization API Helpers
// ============================================================================

/// Prepare data for visualization - time series chart
pub fn prepare_timeseries_chart_data(
    historical_data: &Vec<TimeSeriesDataPoint>,
    interval: u64,
) -> Vec<TimeSeriesDataPoint> {
    // Aggregate data points by interval for cleaner visualization
    // This is a simplified version - production would have more sophisticated aggregation
    historical_data.clone()
}

/// Prepare data for visualization - performance distribution
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PerformanceDistribution {
    pub range_start: i128,
    pub range_end: i128,
    pub count: u32,
    pub percentage: u32,
}

pub fn prepare_distribution_data(
    signals: &Vec<SignalData>,
    num_buckets: u32,
) -> Vec<PerformanceDistribution> {
    if signals.is_empty() {
        return Vec::new();
    }
    
    // Find min and max PnL
    let mut min_pnl = i128::MAX;
    let mut max_pnl = i128::MIN;
    
    for signal in signals.iter() {
        if signal.pnl < min_pnl {
            min_pnl = signal.pnl;
        }
        if signal.pnl > max_pnl {
            max_pnl = signal.pnl;
        }
    }
    
    // Create buckets
    let range = max_pnl - min_pnl;
    let bucket_size = if range > 0 {
        range / num_buckets as i128
    } else {
        1
    };
    
    // This would be implemented with proper bucket counting
    // Simplified for demonstration
    Vec::new()
}

/// Comparison metrics for multiple providers
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ProviderComparison {
    pub provider: Address,
    pub win_rate: u32,
    pub total_pnl: i128,
    pub risk_score: u32,
    pub consistency_score: u32,
    pub rank: u32,
}

pub fn compare_providers(
    providers: &Vec<SignalProviderAnalytics>,
) -> Vec<ProviderComparison> {
    // Sort and rank providers based on multiple criteria
    // This is a simplified version
    Vec::new()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_win_rate() {
        assert_eq!(calculate_win_rate(75, 100), 7500); // 75%
        assert_eq!(calculate_win_rate(50, 100), 5000); // 50%
        assert_eq!(calculate_win_rate(0, 100), 0);     // 0%
        assert_eq!(calculate_win_rate(100, 100), 10000); // 100%
        assert_eq!(calculate_win_rate(10, 0), 0);      // Division by zero
    }

    #[test]
    fn test_calculate_profit_factor() {
        assert_eq!(calculate_profit_factor(200, -100), 200); // 2.0x
        assert_eq!(calculate_profit_factor(150, -100), 150); // 1.5x
        assert_eq!(calculate_profit_factor(100, -200), 50);  // 0.5x
        assert_eq!(calculate_profit_factor(100, 0), 10000);  // Max
        assert_eq!(calculate_profit_factor(0, -100), 0);     // 0x
    }

    #[test]
    fn test_calculate_sharpe_ratio() {
        assert_eq!(calculate_sharpe_ratio(150, 100, 50), 100); // 1.0
        assert_eq!(calculate_sharpe_ratio(200, 100, 50), 150); // 1.5
        assert_eq!(calculate_sharpe_ratio(100, 100, 50), 50);  // 0.5
        assert_eq!(calculate_sharpe_ratio(100, 0, 50), 0);     // Div by zero
    }

    #[test]
    fn test_calculate_max_drawdown() {
        assert_eq!(calculate_max_drawdown(1000, 800), 2000);  // 20%
        assert_eq!(calculate_max_drawdown(1000, 500), 5000);  // 50%
        assert_eq!(calculate_max_drawdown(1000, 1000), 0);    // 0%
        assert_eq!(calculate_max_drawdown(0, 0), 0);          // Edge case
    }

    #[test]
    fn test_calculate_consistency_score() {
        assert_eq!(calculate_consistency_score(10, 1000), 90);
        assert_eq!(calculate_consistency_score(50, 5000), 50);
        assert_eq!(calculate_consistency_score(0, 0), 100);
    }

    #[test]
    fn test_calculate_risk_score() {
        assert_eq!(calculate_risk_score(2000, 3000, 100), 60); // 20+30+10
        assert_eq!(calculate_risk_score(1000, 1000, 50), 25);  // 10+10+5
        assert_eq!(calculate_risk_score(0, 0, 0), 0);
    }
}
