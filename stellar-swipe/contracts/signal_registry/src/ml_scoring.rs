//! Machine Learning-based Signal Quality Scoring
//!
//! Provides ML-powered scoring to predict signal success probability and rank signals by quality.
//! Features extraction, model inference, and continuous learning capabilities.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Map, String, Symbol, Vec};

/// Signal features for ML model
#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalFeatures {
    // Provider features
    pub provider_success_rate: u32,
    pub provider_total_signals: u32,
    pub provider_avg_roi: i128,
    pub provider_consistency: i128,
    pub provider_follower_count: u32,
    
    // Signal features
    pub asset_pair_volatility: i128,
    pub signal_price_vs_current: i128,
    pub rationale_sentiment: i32,
    pub rationale_length: u32,
    pub time_of_day: u32,
    pub day_of_week: u32,
    
    // Market features
    pub market_trend: i32,
    pub market_volume_24h: i128,
    pub asset_rsi: u32,
    pub asset_macd_signal: i32,
    pub overall_market_sentiment: i32,
    
    // Interaction features
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

/// Model performance tracking
#[contracttype]
#[derive(Clone, Debug)]
pub struct ModelPerformance {
    pub model_version: u32,
    pub total_predictions: u32,
    pub correct_predictions: u32,
    pub accuracy: u32,
    pub last_updated: u64,
}

/// Storage keys
#[contracttype]
pub enum MLStorageKey {
    CurrentModel,
    ModelPerformance(u32),
    SignalScore(u64),
    SignalFeatures(u64),
    SignalOutcome(u64),
}

// Constants
const SCALE_FACTOR: i128 = 10_000; // Basis points (0.01%)
const CONFIDENCE_MULTIPLIER: i128 = 150; // 1.5% base confidence
const MIN_SAMPLES_FOR_CONFIDENCE: u32 = 100;

/// ==========================
/// Feature Extraction
/// ==========================

/// Extract features from a signal for ML scoring
pub fn extract_signal_features(
    env: &Env,
    signal_id: u64,
    provider_stats: &ProviderStats,
    market_data: &MarketData,
) -> Result<SignalFeatures, String> {
    let signal = get_signal(env, signal_id)?;
    
    // Provider features
    let provider_success_rate = if provider_stats.total_signals > 0 {
        (provider_stats.successful_signals * 10000) / provider_stats.total_signals
    } else {
        5000 // Default 50% for new providers
    };
    
    let provider_consistency = calculate_consistency(provider_stats);
    
    // Signal features
    let current_price = market_data.current_price;
    let signal_price_vs_current = if current_price > 0 {
        ((signal.price - current_price) * 10000) / current_price
    } else {
        0
    };
    
    let rationale_sentiment = analyze_sentiment(&signal.rationale);
    let rationale_length = signal.rationale.len();
    
    let time_of_day = ((signal.timestamp % 86400) / 3600) as u32;
    let day_of_week = ((signal.timestamp / 86400) % 7) as u32;
    
    // Market features
    let asset_pair_volatility = market_data.volatility_30d;
    let market_trend = market_data.trend_indicator;
    let market_volume_24h = market_data.volume_24h;
    let asset_rsi = market_data.rsi_14;
    let asset_macd_signal = market_data.macd_signal;
    let overall_market_sentiment = market_data.market_sentiment;
    
    // Interaction features
    let provider_expertise_in_asset = count_provider_signals_in_asset(
        env,
        &signal.provider,
        &signal.asset_pair,
    )?;
    
    let signal_uniqueness = calculate_signal_uniqueness(env, &signal)?;
    
    Ok(SignalFeatures {
        provider_success_rate,
        provider_total_signals: provider_stats.total_signals,
        provider_avg_roi: provider_stats.avg_return,
        provider_consistency,
        provider_follower_count: provider_stats.follower_count,
        asset_pair_volatility,
        signal_price_vs_current,
        rationale_sentiment,
        rationale_length,
        time_of_day,
        day_of_week,
        market_trend,
        market_volume_24h,
        asset_rsi,
        asset_macd_signal,
        overall_market_sentiment,
        provider_expertise_in_asset,
        signal_uniqueness,
    })
}

/// ==========================
/// ML Model Scoring
/// ==========================

/// Score a signal using the ML model
pub fn score_signal(
    env: &Env,
    signal_id: u64,
    features: &SignalFeatures,
    model: &MLModel,
) -> Result<SignalScore, String> {
    // Linear combination of features
    let mut score = model.intercept;
    
    // Add weighted features
    score += (features.provider_success_rate as i128) 
        * get_weight(&model.feature_weights, "provider_success_rate");
    score += (features.provider_total_signals as i128) 
        * get_weight(&model.feature_weights, "provider_total_signals");
    score += features.provider_avg_roi 
        * get_weight(&model.feature_weights, "provider_avg_roi");
    score += features.provider_consistency 
        * get_weight(&model.feature_weights, "provider_consistency");
    score += (features.provider_follower_count as i128) 
        * get_weight(&model.feature_weights, "provider_follower_count");
    
    score += features.asset_pair_volatility 
        * get_weight(&model.feature_weights, "asset_pair_volatility");
    score += features.signal_price_vs_current 
        * get_weight(&model.feature_weights, "signal_price_vs_current");
    score += (features.rationale_sentiment as i128) 
        * get_weight(&model.feature_weights, "rationale_sentiment");
    score += (features.rationale_length as i128) 
        * get_weight(&model.feature_weights, "rationale_length");
    score += (features.time_of_day as i128) 
        * get_weight(&model.feature_weights, "time_of_day");
    score += (features.day_of_week as i128) 
        * get_weight(&model.feature_weights, "day_of_week");
    
    score += (features.market_trend as i128) 
        * get_weight(&model.feature_weights, "market_trend");
    score += features.market_volume_24h 
        * get_weight(&model.feature_weights, "market_volume_24h");
    score += (features.asset_rsi as i128) 
        * get_weight(&model.feature_weights, "asset_rsi");
    score += (features.asset_macd_signal as i128) 
        * get_weight(&model.feature_weights, "asset_macd_signal");
    score += (features.overall_market_sentiment as i128) 
        * get_weight(&model.feature_weights, "overall_market_sentiment");
    
    score += (features.provider_expertise_in_asset as i128) 
        * get_weight(&model.feature_weights, "provider_expertise_in_asset");
    score += (features.signal_uniqueness as i128) 
        * get_weight(&model.feature_weights, "signal_uniqueness");
    
    // Apply sigmoid to convert to probability (0-10000 basis points)
    let probability = sigmoid(score);
    
    // Calculate confidence interval
    let confidence = calculate_confidence(features, model);
    
    let confidence_lower = max(0, probability - confidence);
    let confidence_upper = min(SCALE_FACTOR, probability + confidence);
    
    Ok(SignalScore {
        signal_id,
        quality_score: probability,
        success_probability: probability,
        confidence_lower,
        confidence_upper,
        model_version: model.model_version,
        scored_at: env.ledger().timestamp(),
    })
}

/// Get feature weight from model, default to 0 if not found
fn get_weight(weights: &Map<String, i128>, feature_name: &str) -> i128 {
    weights.get(String::from_str(&weights.env(), feature_name))
        .unwrap_or(0)
}
