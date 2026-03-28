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

use core::cmp::{max, min};

/// Caller-supplied provider aggregates for feature extraction.
#[derive(Clone, Default)]
pub struct ProviderStats {
    pub total_signals: u32,
    pub successful_signals: u32,
    pub avg_return: i128,
    pub follower_count: u32,
}

/// Caller-supplied market snapshot for feature extraction.
#[derive(Clone, Default)]
pub struct MarketData {
    pub current_price: i128,
    pub volatility_30d: i128,
    pub trend_indicator: i32,
    pub volume_24h: i128,
    pub rsi_14: u32,
    pub macd_signal: i32,
    pub market_sentiment: i32,
}

fn sigmoid(score: i128) -> i128 {
    let x = score / 100;
    if x <= 0 {
        0
    } else if x >= SCALE_FACTOR {
        SCALE_FACTOR
    } else {
        x
    }
}

fn calculate_confidence(_features: &SignalFeatures, _model: &MLModel) -> i128 {
    CONFIDENCE_MULTIPLIER
}

/// ==========================
/// Feature Extraction
/// ==========================

/// Extract features from a signal for ML scoring.
///
/// Full on-chain extraction is not yet wired to persistent signal storage; callers should
/// build [`SignalFeatures`] off-chain until this returns `Ok`.
pub fn extract_signal_features(
    env: &Env,
    _signal_id: u64,
    _provider_stats: &ProviderStats,
    _market_data: &MarketData,
) -> Result<SignalFeatures, String> {
    Err(String::from_str(
        env,
        "extract_signal_features: on-chain extraction not implemented",
    ))
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
        * get_weight(env, &model.feature_weights, "provider_success_rate");
    score += (features.provider_total_signals as i128) 
        * get_weight(env, &model.feature_weights, "provider_total_signals");
    score += features.provider_avg_roi 
        * get_weight(env, &model.feature_weights, "provider_avg_roi");
    score += features.provider_consistency 
        * get_weight(env, &model.feature_weights, "provider_consistency");
    score += (features.provider_follower_count as i128) 
        * get_weight(env, &model.feature_weights, "provider_follower_count");
    
    score += features.asset_pair_volatility 
        * get_weight(env, &model.feature_weights, "asset_pair_volatility");
    score += features.signal_price_vs_current 
        * get_weight(env, &model.feature_weights, "signal_price_vs_current");
    score += (features.rationale_sentiment as i128) 
        * get_weight(env, &model.feature_weights, "rationale_sentiment");
    score += (features.rationale_length as i128) 
        * get_weight(env, &model.feature_weights, "rationale_length");
    score += (features.time_of_day as i128) 
        * get_weight(env, &model.feature_weights, "time_of_day");
    score += (features.day_of_week as i128) 
        * get_weight(env, &model.feature_weights, "day_of_week");
    
    score += (features.market_trend as i128) 
        * get_weight(env, &model.feature_weights, "market_trend");
    score += features.market_volume_24h 
        * get_weight(env, &model.feature_weights, "market_volume_24h");
    score += (features.asset_rsi as i128) 
        * get_weight(env, &model.feature_weights, "asset_rsi");
    score += (features.asset_macd_signal as i128) 
        * get_weight(env, &model.feature_weights, "asset_macd_signal");
    score += (features.overall_market_sentiment as i128) 
        * get_weight(env, &model.feature_weights, "overall_market_sentiment");
    
    score += (features.provider_expertise_in_asset as i128) 
        * get_weight(env, &model.feature_weights, "provider_expertise_in_asset");
    score += (features.signal_uniqueness as i128) 
        * get_weight(env, &model.feature_weights, "signal_uniqueness");
    
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
fn get_weight(env: &Env, weights: &Map<String, i128>, feature_name: &str) -> i128 {
    weights
        .get(String::from_str(env, feature_name))
        .unwrap_or(0)
}
