# Advanced Analytics Engine for Signal Performance

## Overview

The Advanced Analytics Engine provides deep insights into signal provider performance through comprehensive data analysis, historical trend analysis, predictive analytics, and anomaly detection.

---

## Table of Contents

1. [Architecture](#architecture)
2. [Data Models](#data-models)
3. [Performance Metrics](#performance-metrics)
4. [Historical Trend Analysis](#historical-trend-analysis)
5. [Predictive Analytics](#predictive-analytics)
6. [Anomaly Detection](#anomaly-detection)
7. [Performance Reports](#performance-reports)
8. [Data Visualization APIs](#data-visualization-apis)
9. [Usage Examples](#usage-examples)

---

## Architecture

### System Components

```
┌─────────────────────────────────────────────────────────┐
│           Advanced Analytics Engine                      │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐ │
│  │   Data       │  │  Performance │  │  Historical  │ │
│  │   Models     │  │   Metrics    │  │    Trends    │ │
│  └──────────────┘  └──────────────┘  └──────────────┘ │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐ │
│  │  Predictive  │  │   Anomaly    │  │  Performance │ │
│  │  Analytics   │  │  Detection   │  │   Reports    │ │
│  └──────────────┘  └──────────────┘  └──────────────┘ │
│                                                          │
│  ┌──────────────────────────────────────────────────┐  │
│  │         Data Visualization APIs                   │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Key Features

- **Comprehensive Metrics**: 15+ performance indicators
- **Historical Analysis**: Trend detection and pattern recognition
- **Predictive Models**: Future performance predictions
- **Anomaly Detection**: 6 types of anomaly detection
- **Rich Reports**: Detailed performance reports
- **Visualization Ready**: APIs for chart and graph generation

---

## Data Models

### SignalProviderAnalytics

Core analytics data structure for signal providers.

```rust
pub struct SignalProviderAnalytics {
    pub provider: Address,
    pub total_signals: u32,
    pub successful_signals: u32,
    pub failed_signals: u32,
    pub total_profit: i128,
    pub total_loss: i128,
    pub avg_profit_per_signal: i128,
    pub win_rate: u32,              // 0-10000 (0.00% - 100.00%)
    pub profit_factor: u32,         // Ratio * 100
    pub sharpe_ratio: i32,          // Ratio * 100
    pub max_drawdown: u32,          // Percentage * 100
    pub avg_holding_period: u64,    // Seconds
    pub consistency_score: u32,     // 0-100
    pub risk_score: u32,            // 0-100
    pub last_updated: u64,
}
```

**Key Metrics**:
- **Win Rate**: Percentage of successful signals (basis points)
- **Profit Factor**: Ratio of total profit to total loss
- **Sharpe Ratio**: Risk-adjusted return metric
- **Max Drawdown**: Largest peak-to-trough decline
- **Consistency Score**: Performance stability (0-100)
- **Risk Score**: Overall risk assessment (0-100)

### TimeSeriesDataPoint

Historical data point for trend analysis.

```rust
pub struct TimeSeriesDataPoint {
    pub timestamp: u64,
    pub value: i128,
    pub signal_count: u32,
    pub win_rate: u32,
}
```

### PeriodPerformance

Performance metrics over a specific time period.

```rust
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
```

---

## Performance Metrics

### Win Rate Calculation

```rust
pub fn calculate_win_rate(successful: u32, total: u32) -> u32
```

**Formula**: `(successful / total) * 10000`

**Returns**: Basis points (0-10000 representing 0.00% - 100.00%)

**Example**:
- 75 successful out of 100 total = 7500 (75.00%)
- 50 successful out of 100 total = 5000 (50.00%)

### Profit Factor

```rust
pub fn calculate_profit_factor(total_profit: i128, total_loss: i128) -> u32
```

**Formula**: `(total_profit / |total_loss|) * 100`

**Interpretation**:
- > 200 (2.0x): Excellent
- 150-200 (1.5-2.0x): Good
- 100-150 (1.0-1.5x): Acceptable
- < 100 (< 1.0x): Poor

**Example**:
- Profit: $200, Loss: $100 → 200 (2.0x profit factor)

### Sharpe Ratio

```rust
pub fn calculate_sharpe_ratio(
    avg_return: i128,
    std_deviation: i128,
    risk_free_rate: i128,
) -> i32
```

**Formula**: `((avg_return - risk_free_rate) / std_deviation) * 100`

**Interpretation**:
- > 200 (2.0): Excellent
- 100-200 (1.0-2.0): Good
- 50-100 (0.5-1.0): Acceptable
- < 50 (< 0.5): Poor

### Maximum Drawdown

```rust
pub fn calculate_max_drawdown(peak_value: i128, trough_value: i128) -> u32
```

**Formula**: `((peak - trough) / peak) * 10000`

**Returns**: Basis points

**Interpretation**:
- < 1000 (10%): Low risk
- 1000-2000 (10-20%): Moderate risk
- 2000-3000 (20-30%): High risk
- > 3000 (>30%): Very high risk

### Consistency Score

```rust
pub fn calculate_consistency_score(
    win_rate_variance: u32,
    return_variance: i128,
) -> u32
```

**Returns**: Score 0-100 (higher is better)

**Factors**:
- Win rate stability
- Return variance
- Performance predictability

### Risk Score

```rust
pub fn calculate_risk_score(
    max_drawdown: u32,
    volatility: u32,
    leverage_used: u32,
) -> u32
```

**Returns**: Score 0-100 (higher is riskier)

**Components**:
- Drawdown component (0-40)
- Volatility component (0-40)
- Leverage component (0-20)

---

## Historical Trend Analysis

### Trend Detection

```rust
pub fn analyze_historical_trend(
    data_points: &Vec<TimeSeriesDataPoint>,
) -> TrendDirection
```

**Trend Classifications**:
- **StrongUptrend**: >20% improvement
- **Uptrend**: 5-20% improvement
- **Sideways**: -5% to +5%
- **Downtrend**: -20% to -5% decline
- **StrongDowntrend**: <-20% decline

**Algorithm**:
1. Calculate recent average (last 5 data points)
2. Calculate older average (previous 5 data points)
3. Compare percentage difference
4. Classify trend based on thresholds

### Volatility Calculation

```rust
pub fn calculate_volatility(data_points: &Vec<TimeSeriesDataPoint>) -> u32
```

**Algorithm**:
1. Calculate mean of all values
2. Calculate variance (sum of squared differences)
3. Return approximate square root (standard deviation)

**Interpretation**:
- < 1000: Low volatility
- 1000-3000: Moderate volatility
- 3000-5000: High volatility
- > 5000: Very high volatility

---

## Predictive Analytics

### Prediction Generation

```rust
pub fn generate_predictions(
    analytics: &SignalProviderAnalytics,
    historical_data: &Vec<TimeSeriesDataPoint>,
) -> PredictiveAnalytics
```

**Output**:
```rust
pub struct PredictiveAnalytics {
    pub provider: Address,
    pub predicted_win_rate: u32,
    pub confidence_level: u32,      // 0-100
    pub trend_direction: TrendDirection,
    pub risk_level: RiskLevel,
    pub recommendation: Recommendation,
}
```

### Win Rate Prediction

**Formula**:
```
predicted_win_rate = current_win_rate + (trend_adjustment * consistency / 100)
```

**Trend Adjustments**:
- StrongUptrend: +500 basis points
- Uptrend: +200 basis points
- Sideways: 0
- Downtrend: -200 basis points
- StrongDowntrend: -500 basis points

### Confidence Calculation

**Factors**:
- Data quantity (more data = higher confidence)
- Consistency score (stable performance = higher confidence)

**Formula**:
```
confidence = min(data_points * 2, 50) + (consistency / 2)
```

### Risk Classification

| Risk Score | Classification |
|------------|----------------|
| 0-20 | Very Low |
| 21-40 | Low |
| 41-60 | Medium |
| 61-80 | High |
| 81-100 | Very High |

### Recommendation System

**Factors**:
- Predicted win rate
- Risk level
- Trend direction

**Recommendations**:
- **Strong Buy**: High win rate + low risk + uptrend
- **Buy**: Good win rate + acceptable risk
- **Hold**: Moderate metrics
- **Sell**: Poor win rate or high risk
- **Strong Sell**: Very poor metrics + downtrend

---

## Anomaly Detection

### Anomaly Types

```rust
pub enum AnomalyType {
    SuddenPerformanceDrop,
    UnusuallyHighWinRate,
    SuspiciousPattern,
    VolatilitySpike,
    DrawdownExceeded,
    InactivityPeriod,
}
```

### Detection Algorithms

#### 1. Sudden Performance Drop

**Trigger**: >30% decline in recent vs. older average

**Severity**: Percentage of decline (0-100)

**Algorithm**:
```rust
recent_avg = average(last 5 data points)
older_avg = average(previous 5 data points)
drop_percentage = ((older_avg - recent_avg) / older_avg) * 100

if drop_percentage > 30:
    flag_anomaly(severity = drop_percentage)
```

#### 2. Unusually High Win Rate

**Trigger**: Win rate >95% with >20 signals

**Severity**: 70 (high)

**Rationale**: Extremely high win rates may indicate:
- Data manipulation
- Cherry-picking signals
- Unrealistic backtesting

#### 3. Volatility Spike

**Trigger**: Volatility >5000

**Severity**: volatility / 100 (capped at 100)

**Indicates**: Unstable performance or risky strategy

#### 4. Excessive Drawdown

**Trigger**: Max drawdown >50%

**Severity**: drawdown / 100 (capped at 100)

**Indicates**: High risk or poor risk management

### Anomaly Detection Result

```rust
pub struct AnomalyDetection {
    pub provider: Address,
    pub anomaly_type: AnomalyType,
    pub severity: u32,              // 0-100
    pub detected_at: u64,
    pub description: String,
}
```

---

## Performance Reports

### Report Generation

```rust
pub fn generate_performance_report(
    env: &Env,
    provider: Address,
    analytics: SignalProviderAnalytics,
    period: PeriodPerformance,
    historical_data: Vec<TimeSeriesDataPoint>,
) -> PerformanceReport
```

### Report Structure

```rust
pub struct PerformanceReport {
    pub provider: Address,
    pub report_period: PeriodPerformance,
    pub analytics: SignalProviderAnalytics,
    pub historical_trend: Vec<TimeSeriesDataPoint>,
    pub predictions: PredictiveAnalytics,
    pub anomalies: Vec<AnomalyDetection>,
    pub generated_at: u64,
}
```

### Report Components

1. **Period Performance**: Metrics for specific time range
2. **Overall Analytics**: Cumulative performance data
3. **Historical Trend**: Time-series data for visualization
4. **Predictions**: Future performance forecasts
5. **Anomalies**: Detected issues or concerns
6. **Metadata**: Generation timestamp

---

## Data Visualization APIs

### Time Series Chart Data

```rust
pub fn prepare_timeseries_chart_data(
    historical_data: &Vec<TimeSeriesDataPoint>,
    interval: u64,
) -> Vec<TimeSeriesDataPoint>
```

**Use Case**: Line charts showing performance over time

**Output**: Aggregated data points at specified intervals

### Performance Distribution

```rust
pub struct PerformanceDistribution {
    pub range_start: i128,
    pub range_end: i128,
    pub count: u32,
    pub percentage: u32,
}

pub fn prepare_distribution_data(
    signals: &Vec<SignalData>,
    num_buckets: u32,
) -> Vec<PerformanceDistribution>
```

**Use Case**: Histogram showing PnL distribution

**Output**: Bucketed data for histogram visualization

### Provider Comparison

```rust
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
) -> Vec<ProviderComparison>
```

**Use Case**: Comparative analysis of multiple providers

**Output**: Ranked list with key metrics

---

## Usage Examples

### Example 1: Calculate Basic Metrics

```rust
use analytics_engine::*;

// Calculate win rate
let win_rate = calculate_win_rate(75, 100);
// Result: 7500 (75.00%)

// Calculate profit factor
let profit_factor = calculate_profit_factor(200_000, -100_000);
// Result: 200 (2.0x)

// Calculate Sharpe ratio
let sharpe = calculate_sharpe_ratio(150, 100, 50);
// Result: 100 (1.0)

// Calculate max drawdown
let drawdown = calculate_max_drawdown(1000_000, 800_000);
// Result: 2000 (20.00%)
```

### Example 2: Analyze Historical Trends

```rust
// Create historical data
let mut data_points = Vec::new(&env);
data_points.push_back(TimeSeriesDataPoint {
    timestamp: 1000,
    value: 100_000,
    signal_count: 10,
    win_rate: 7000,
});
// ... add more data points

// Analyze trend
let trend = analyze_historical_trend(&data_points);
// Result: TrendDirection::Uptrend

// Calculate volatility
let volatility = calculate_volatility(&data_points);
// Result: 1500 (moderate volatility)
```

### Example 3: Generate Predictions

```rust
// Create analytics data
let analytics = SignalProviderAnalytics {
    provider: provider_address,
    total_signals: 100,
    successful_signals: 75,
    win_rate: 7500,
    consistency_score: 80,
    risk_score: 30,
    // ... other fields
};

// Generate predictions
let predictions = generate_predictions(&analytics, &historical_data);

// Access prediction results
match predictions.recommendation {
    Recommendation::StrongBuy => {
        // High confidence buy signal
    },
    Recommendation::Buy => {
        // Moderate buy signal
    },
    // ... handle other cases
}
```

### Example 4: Detect Anomalies

```rust
// Detect anomalies
let anomalies = detect_anomalies(&env, &analytics, &historical_data);

// Process detected anomalies
for anomaly in anomalies.iter() {
    match anomaly.anomaly_type {
        AnomalyType::SuddenPerformanceDrop => {
            // Alert: Performance declined significantly
            if anomaly.severity > 50 {
                // Take action for severe drop
            }
        },
        AnomalyType::UnusuallyHighWinRate => {
            // Flag for review: Suspiciously high win rate
        },
        // ... handle other anomaly types
    }
}
```

### Example 5: Generate Performance Report

```rust
// Calculate period performance
let period = calculate_period_performance(
    period_start,
    period_end,
    &signals,
);

// Generate comprehensive report
let report = generate_performance_report(
    &env,
    provider_address,
    analytics,
    period,
    historical_data,
);

// Access report components
let win_rate = report.analytics.win_rate;
let prediction = report.predictions.predicted_win_rate;
let anomaly_count = report.anomalies.len();
```

---

## Performance Considerations

### Computational Complexity

| Operation | Complexity | Notes |
|-----------|------------|-------|
| Win Rate Calculation | O(1) | Simple division |
| Profit Factor | O(1) | Simple division |
| Sharpe Ratio | O(1) | Simple calculation |
| Trend Analysis | O(n) | Linear scan of data points |
| Volatility Calculation | O(n) | Two passes over data |
| Anomaly Detection | O(n) | Multiple linear scans |
| Report Generation | O(n) | Combines multiple O(n) operations |

### Optimization Tips

1. **Batch Processing**: Process multiple providers in single transaction
2. **Caching**: Cache frequently accessed analytics data
3. **Incremental Updates**: Update metrics incrementally rather than recalculating
4. **Data Pruning**: Limit historical data to relevant time windows
5. **Lazy Evaluation**: Calculate expensive metrics only when needed

### Storage Considerations

**Per Provider Storage**:
- Analytics: ~200 bytes
- Historical data (100 points): ~4KB
- Anomalies (10): ~1KB
- Total: ~5KB per provider

**Recommendations**:
- Store only recent historical data (e.g., last 90 days)
- Archive older data off-chain
- Implement data retention policies

---

## Integration Guide

### Step 1: Import Module

```rust
use crate::analytics_engine::*;
```

### Step 2: Initialize Analytics

```rust
let analytics = SignalProviderAnalytics {
    provider: provider_address,
    total_signals: 0,
    successful_signals: 0,
    failed_signals: 0,
    total_profit: 0,
    total_loss: 0,
    avg_profit_per_signal: 0,
    win_rate: 0,
    profit_factor: 0,
    sharpe_ratio: 0,
    max_drawdown: 0,
    avg_holding_period: 0,
    consistency_score: 0,
    risk_score: 0,
    last_updated: env.ledger().timestamp(),
};
```

### Step 3: Update on Signal Completion

```rust
// When signal completes
analytics.total_signals += 1;

if signal_pnl > 0 {
    analytics.successful_signals += 1;
    analytics.total_profit += signal_pnl;
} else {
    analytics.failed_signals += 1;
    analytics.total_loss += signal_pnl;
}

// Recalculate metrics
analytics.win_rate = calculate_win_rate(
    analytics.successful_signals,
    analytics.total_signals,
);

analytics.profit_factor = calculate_profit_factor(
    analytics.total_profit,
    analytics.total_loss,
);

// Update timestamp
analytics.last_updated = env.ledger().timestamp();
```

### Step 4: Generate Reports Periodically

```rust
// Generate monthly report
let report = generate_performance_report(
    &env,
    provider_address,
    analytics,
    period_performance,
    historical_data,
);

// Store or emit report
env.events().publish(("performance_report",), report);
```

---

## Best Practices

### Data Quality

1. **Validate Inputs**: Ensure all input data is valid and within expected ranges
2. **Handle Edge Cases**: Check for division by zero, empty datasets, etc.
3. **Sanitize Data**: Remove outliers that could skew metrics
4. **Timestamp Accuracy**: Use consistent timestamp sources

### Metric Interpretation

1. **Context Matters**: Consider market conditions when interpreting metrics
2. **Multiple Metrics**: Don't rely on single metric; use combination
3. **Time Frames**: Analyze multiple time frames (daily, weekly, monthly)
4. **Peer Comparison**: Compare against similar providers

### Anomaly Handling

1. **Investigate Promptly**: Review anomalies quickly
2. **False Positives**: Expect some false positives; verify before action
3. **Severity Levels**: Prioritize by severity score
4. **Historical Context**: Consider if anomaly is truly unusual

### Performance Optimization

1. **Batch Updates**: Update multiple providers together
2. **Lazy Loading**: Load historical data only when needed
3. **Cache Results**: Cache expensive calculations
4. **Prune Old Data**: Remove outdated historical data

---

## Future Enhancements

### Planned Features

1. **Machine Learning Integration**: Advanced prediction models
2. **Sentiment Analysis**: Incorporate market sentiment
3. **Cross-Provider Correlation**: Analyze provider relationships
4. **Real-time Alerts**: Instant anomaly notifications
5. **Custom Metrics**: User-defined performance indicators
6. **Backtesting Engine**: Historical strategy testing
7. **Risk Management Tools**: Position sizing recommendations
8. **Portfolio Analytics**: Multi-provider portfolio analysis

### Research Areas

1. **Advanced Prediction Models**: LSTM, ARIMA, Prophet
2. **Clustering Algorithms**: Group similar providers
3. **Outlier Detection**: More sophisticated anomaly detection
4. **Causal Analysis**: Identify performance drivers
5. **Network Analysis**: Provider influence mapping

---

## Conclusion

The Advanced Analytics Engine provides comprehensive tools for analyzing signal provider performance. By combining historical analysis, predictive modeling, and anomaly detection, it enables data-driven decision making for signal selection and risk management.

**Key Benefits**:
- ✅ Comprehensive performance metrics
- ✅ Historical trend analysis
- ✅ Predictive analytics
- ✅ Automated anomaly detection
- ✅ Rich performance reports
- ✅ Visualization-ready APIs

**Document Version**: 1.0.0  
**Last Updated**: 2026-06-01  
**Module**: `contracts/signal_registry/src/analytics_engine.rs`
