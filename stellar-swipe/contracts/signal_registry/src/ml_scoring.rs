//! ML scoring types (feature extraction / scoring pipeline is deferred).

use soroban_sdk::{contracttype, Map, String};

/// Signal features for ML model
#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalFeatures {
    pub provider_success_rate: u32,
    pub provider_total_signals: u32,
    pub provider_avg_roi: i128,
    pub provider_consistency: i128,
    pub provider_follower_count: u32,
    pub asset_pair_volatility: i128,
    pub signal_price_vs_current: i128,
    pub rationale_sentiment: i32,
    pub rationale_length: u32,
    pub time_of_day: u32,
    pub day_of_week: u32,
    pub market_trend: i32,
    pub market_volume_24h: i128,
    pub asset_rsi: u32,
    pub asset_macd_signal: i32,
    pub overall_market_sentiment: i32,
    pub provider_expertise_in_asset: u32,
    pub signal_uniqueness: u32,
}

/// ML model for signal scoring
#[contracttype]
#[derive(Clone, Debug)]
pub struct MLModel {
    pub feature_weights: Map<String, i128>,
    pub intercept: i128,
    pub model_version: u32,
    pub training_date: u64,
    pub accuracy: u32,
    pub sample_count: u32,
}

/// Signal quality score with confidence intervals
#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalScore {
    pub signal_id: u64,
    pub quality_score: i128,
    pub success_probability: i128,
    pub confidence_lower: i128,
    pub confidence_upper: i128,
    pub model_version: u32,
    pub scored_at: u64,
}
