# Analytics Engine API Reference

## Overview

Complete API reference for the Advanced Analytics Engine.

---

## Core Functions

### Performance Metrics

#### calculate_win_rate
```rust
pub fn calculate_win_rate(successful: u32, total: u32) -> u32
```
**Parameters**:
- `successful`: Number of successful signals
- `total`: Total number of signals

**Returns**: Win rate in basis points (0-10000)

**Example**:
```rust
let win_rate = calculate_win_rate(75, 100); // 7500 (75%)
```

---

#### calculate_profit_factor
```rust
pub fn calculate_profit_factor(total_profit: i128, total_loss: i128) -> u32
```
**Parameters**:
- `total_profit`: Total profit amount
- `total_loss`: Total loss amount (negative)

**Returns**: Profit factor * 100

**Example**:
```rust
let pf = calculate_profit_factor(200_000, -100_000); // 200 (2.0x)
```

---

#### calculate_sharpe_ratio
```rust
pub fn calculate_sharpe_ratio(
    avg_return: i128,
    std_deviation: i128,
    risk_free_rate: i128,
) -> i32
```
**Parameters**:
- `avg_return`: Average return
- `std_deviation`: Standard deviation of returns
- `risk_free_rate`: Risk-free rate of return

**Returns**: Sharpe ratio * 100

---

#### calculate_max_drawdown
```rust
pub fn calculate_max_drawdown(peak_value: i128, trough_value: i128) -> u32
```
**Parameters**:
- `peak_value`: Peak portfolio value
- `trough_value`: Trough portfolio value

**Returns**: Drawdown in basis points

---

#### calculate_consistency_score
```rust
pub fn calculate_consistency_score(
    win_rate_variance: u32,
    return_variance: i128,
) -> u32
```
**Parameters**:
- `win_rate_variance`: Variance in win rate
- `return_variance`: Variance in returns

**Returns**: Consistency score (0-100)

---

#### calculate_risk_score
```rust
pub fn calculate_risk_score(
    max_drawdown: u32,
    volatility: u32,
    leverage_used: u32,
) -> u32
```
**Parameters**:
- `max_drawdown`: Maximum drawdown
- `volatility`: Performance volatility
- `leverage_used`: Leverage factor

**Returns**: Risk score (0-100)

---

### Historical Analysis

#### analyze_historical_trend
```rust
pub fn analyze_historical_trend(
    data_points: &Vec<TimeSeriesDataPoint>,
) -> TrendDirection
```
**Parameters**:
- `data_points`: Historical time series data

**Returns**: `TrendDirection` enum

**Possible Values**:
- `StrongUptrend`
- `Uptrend`
- `Sideways`
- `Downtrend`
- `StrongDowntrend`

---

#### calculate_volatility
```rust
pub fn calculate_volatility(data_points: &Vec<TimeSeriesDataPoint>) -> u32
```
**Parameters**:
- `data_points`: Historical time series data

**Returns**: Volatility measure

---

### Predictive Analytics

#### generate_predictions
```rust
pub fn generate_predictions(
    analytics: &SignalProviderAnalytics,
    historical_data: &Vec<TimeSeriesDataPoint>,
) -> PredictiveAnalytics
```
**Parameters**:
- `analytics`: Current analytics data
- `historical_data`: Historical performance data

**Returns**: `PredictiveAnalytics` struct with predictions

---

### Anomaly Detection

#### detect_anomalies
```rust
pub fn detect_anomalies(
    env: &Env,
    analytics: &SignalProviderAnalytics,
    historical_data: &Vec<TimeSeriesDataPoint>,
) -> Vec<AnomalyDetection>
```
**Parameters**:
- `env`: Soroban environment
- `analytics`: Current analytics data
- `historical_data`: Historical performance data

**Returns**: Vector of detected anomalies

---

### Report Generation

#### generate_performance_report
```rust
pub fn generate_performance_report(
    env: &Env,
    provider: Address,
    analytics: SignalProviderAnalytics,
    period: PeriodPerformance,
    historical_data: Vec<TimeSeriesDataPoint>,
) -> PerformanceReport
```
**Parameters**:
- `env`: Soroban environment
- `provider`: Provider address
- `analytics`: Analytics data
- `period`: Period performance data
- `historical_data`: Historical data points

**Returns**: Complete `PerformanceReport`

---

#### calculate_period_performance
```rust
pub fn calculate_period_performance(
    period_start: u64,
    period_end: u64,
    signals: &Vec<SignalData>,
) -> PeriodPerformance
```
**Parameters**:
- `period_start`: Start timestamp
- `period_end`: End timestamp
- `signals`: Signal data for period

**Returns**: `PeriodPerformance` struct

---

### Visualization APIs

#### prepare_timeseries_chart_data
```rust
pub fn prepare_timeseries_chart_data(
    historical_data: &Vec<TimeSeriesDataPoint>,
    interval: u64,
) -> Vec<TimeSeriesDataPoint>
```
**Parameters**:
- `historical_data`: Raw historical data
- `interval`: Aggregation interval

**Returns**: Aggregated data for charts

---

#### prepare_distribution_data
```rust
pub fn prepare_distribution_data(
    signals: &Vec<SignalData>,
    num_buckets: u32,
) -> Vec<PerformanceDistribution>
```
**Parameters**:
- `signals`: Signal data
- `num_buckets`: Number of histogram buckets

**Returns**: Distribution data for histograms

---

#### compare_providers
```rust
pub fn compare_providers(
    providers: &Vec<SignalProviderAnalytics>,
) -> Vec<ProviderComparison>
```
**Parameters**:
- `providers`: List of provider analytics

**Returns**: Ranked comparison data

---

## Data Structures

### SignalProviderAnalytics
```rust
pub struct SignalProviderAnalytics {
    pub provider: Address,
    pub total_signals: u32,
    pub successful_signals: u32,
    pub failed_signals: u32,
    pub total_profit: i128,
    pub total_loss: i128,
    pub avg_profit_per_signal: i128,
    pub win_rate: u32,
    pub profit_factor: u32,
    pub sharpe_ratio: i32,
    pub max_drawdown: u32,
    pub avg_holding_period: u64,
    pub consistency_score: u32,
    pub risk_score: u32,
    pub last_updated: u64,
}
```

### TimeSeriesDataPoint
```rust
pub struct TimeSeriesDataPoint {
    pub timestamp: u64,
    pub value: i128,
    pub signal_count: u32,
    pub win_rate: u32,
}
```

### PeriodPerformance
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

### PredictiveAnalytics
```rust
pub struct PredictiveAnalytics {
    pub provider: Address,
    pub predicted_win_rate: u32,
    pub confidence_level: u32,
    pub trend_direction: TrendDirection,
    pub risk_level: RiskLevel,
    pub recommendation: Recommendation,
}
```

### AnomalyDetection
```rust
pub struct AnomalyDetection {
    pub provider: Address,
    pub anomaly_type: AnomalyType,
    pub severity: u32,
    pub detected_at: u64,
    pub description: String,
}
```

### PerformanceReport
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

---

## Enums

### TrendDirection
```rust
pub enum TrendDirection {
    StrongUptrend,
    Uptrend,
    Sideways,
    Downtrend,
    StrongDowntrend,
}
```

### RiskLevel
```rust
pub enum RiskLevel {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}
```

### Recommendation
```rust
pub enum Recommendation {
    StrongBuy,
    Buy,
    Hold,
    Sell,
    StrongSell,
}
```

### AnomalyType
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

---

**Document Version**: 1.0.0  
**Last Updated**: 2026-06-01
