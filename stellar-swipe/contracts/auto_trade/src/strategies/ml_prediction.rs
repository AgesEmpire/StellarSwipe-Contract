//! ML Prediction Strategy
//!
//! Deploys lightweight ML models on-chain for real-time prediction and automated trading decisions.
//! Supports logistic regression, decision trees, random forest ensembles, and compact neural nets.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};

use crate::errors::AutoTradeError;
use crate::iceberg::AssetPair;

const PRECISION: i128 = 10_000;
const MAX_LOGISTIC_WEIGHTS: u32 = 50;
const MAX_TREE_NODES: u32 = 1_000;
const MAX_FOREST_TREES: u32 = 100;
const MAX_NN_LAYERS: u32 = 10;
const MAX_LAYER_NEURONS: u32 = 100;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeDirection {
    Buy,
    Sell,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MLTradingStrategy {
    pub strategy_id: u64,
    pub user: Address,
    pub asset_pair: AssetPair,
    pub model: MLModel,
    pub feature_config: FeatureConfig,
    pub prediction_threshold: u32, // Min prediction confidence to trade
    pub position_size_pct: u32,
    /// `position_id == 0` means no active position (avoids `Option` in contract types for SDK testutils).
    pub active_position: MLPosition,
    pub model_version: u32,
    pub last_model_update: u64,
}

#[allow(dead_code)]
fn ml_position_absent() -> MLPosition {
    MLPosition {
        position_id: 0,
        predicted_direction: TradeDirection::Buy,
        prediction_confidence: 0,
        entry_price: 0,
        amount: 0,
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MLModel {
    LogisticRegression(Vec<i128>, i128),
    DecisionTree(Vec<TreeNode>),
    RandomForest(Vec<DecisionTree>, Vec<u32>),
    NeuralNetwork(Vec<Layer>),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionTree {
    pub nodes: Vec<TreeNode>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    pub feature_index: u32,
    pub threshold: i128,
    pub left_child: Option<u32>,
    pub right_child: Option<u32>,
    pub leaf_value: Option<i128>, // Prediction if leaf node
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub weights: Vec<Vec<i128>>,
    pub biases: Vec<i128>,
    pub activation: ActivationFunction,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationFunction {
    ReLU,
    Sigmoid,
    Tanh,
    Linear,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureConfig {
    pub features: Vec<FeatureType>,
    pub normalization: Map<String, NormalizationParams>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizationParams {
    pub min: i128,
    pub max: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureType {
    Price,
    Volume,
    RSI,
    MACD,
    BollingerBands,
    PriceChange(u32),
    VolumeChange(u32),
    Custom(String, String),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MLPosition {
    pub position_id: u64,
    pub predicted_direction: TradeDirection,
    pub prediction_confidence: u32,
    pub entry_price: i128,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MLPrediction {
    pub direction: TradeDirection,
    pub confidence: u32,
    pub raw_score: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MLSignal {
    pub direction: TradeDirection,
    pub confidence: u32,
    pub features: Vec<i128>,
    pub model_version: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MLModelPerformance {
    pub model_version: u32,
    pub total_predictions: u32,
    pub accurate_predictions: u32,
    pub false_positives: u32,
    pub false_negatives: u32,
    pub avg_confidence_on_correct: u32,
    pub avg_confidence_on_incorrect: u32,
    pub sharpe_ratio: i32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredictionRecord {
    pub confidence: u32,
    pub was_correct: bool,
}

#[contracttype]
pub enum MLDataKey {
    Strategy(u64),
    Performance(u64),
    PositionCounter,
}

/// Extract normalized features based on config.
pub fn extract_features(
    env: &Env,
    asset_pair: AssetPair,
    feature_config: &FeatureConfig,
) -> Result<Vec<i128>, AutoTradeError> {
    let mut features = Vec::new(env);

    for i in 0..feature_config.features.len() {
        let feature_type = feature_config.features.get(i).unwrap();

        let value = match &feature_type {
            FeatureType::Price => get_current_price(asset_pair.clone())?,
            FeatureType::Volume => get_24h_volume(asset_pair.clone())?,
            FeatureType::RSI => calculate_rsi(asset_pair.clone(), 14)? as i128,
            FeatureType::MACD => {
                let (macd, signal) = calculate_macd(asset_pair.clone())?;
                macd - signal
            }
            FeatureType::BollingerBands => {
                let (upper, lower, _) = calculate_bollinger_bands(asset_pair.clone(), 20, 2)?;
                if upper <= lower {
                    return Err(AutoTradeError::InvalidPriceData);
                }
                let current_price = get_current_price(asset_pair.clone())?;
                ((current_price - lower) * PRECISION) / (upper - lower)
            }
            FeatureType::PriceChange(periods) => {
                let current = get_current_price(asset_pair.clone())?;
                let past = get_price_n_periods_ago(asset_pair.clone(), *periods)?;
                if past == 0 {
                    return Err(AutoTradeError::InvalidPriceData);
                }
                ((current - past) * PRECISION) / past
            }
            FeatureType::VolumeChange(periods) => {
                let current = get_current_volume(asset_pair.clone())?;
                let past = get_volume_n_periods_ago(asset_pair.clone(), *periods)?;
                if past == 0 {
                    return Err(AutoTradeError::InvalidPriceData);
                }
                ((current - past) * PRECISION) / past
            }
            FeatureType::Custom(name, calculation) => {
                calculate_custom_feature(asset_pair.clone(), name.clone(), calculation.clone())?
            }
        };

        let normalized = normalize_feature(env, value, &feature_type, feature_config)?;
        features.push_back(normalized);
    }

    Ok(features)
}

fn normalize_feature(
    env: &Env,
    value: i128,
    feature_type: &FeatureType,
    config: &FeatureConfig,
) -> Result<i128, AutoTradeError> {
    let feature_name = feature_type_name(env, feature_type);

    if let Some(params) = config.normalization.get(feature_name) {
        let range = params.max - params.min;
        if range <= 0 {
            return Err(AutoTradeError::InvalidPriceData);
        }
        let normalized = ((value - params.min) * PRECISION) / range;
        Ok(clamp_i128(normalized, 0, PRECISION))
    } else {
        Ok(value)
    }
}

fn feature_type_name(env: &Env, feature_type: &FeatureType) -> String {
    match feature_type {
        FeatureType::Price => String::from_str(env, "price"),
        FeatureType::Volume => String::from_str(env, "volume"),
        FeatureType::RSI => String::from_str(env, "rsi"),
        FeatureType::MACD => String::from_str(env, "macd"),
        FeatureType::BollingerBands => String::from_str(env, "bollinger_bands"),
        FeatureType::PriceChange(_) => String::from_str(env, "price_change"),
        FeatureType::VolumeChange(_) => String::from_str(env, "volume_change"),
        FeatureType::Custom(name, _) => name.clone(),
    }
}

/// Dispatch prediction by model type.
pub fn predict_with_model(
    model: &MLModel,
    features: &Vec<i128>,
) -> Result<MLPrediction, AutoTradeError> {
    match model {
        MLModel::LogisticRegression(weights, intercept) => {
            predict_logistic_regression(features, weights, *intercept)
        }
        MLModel::DecisionTree(nodes) => predict_decision_tree(features, nodes),
        MLModel::RandomForest(trees, tree_weights) => {
            predict_random_forest(features, trees, tree_weights)
        }
        MLModel::NeuralNetwork(layers) => predict_neural_network(features, layers),
    }
}

fn predict_logistic_regression(
    features: &Vec<i128>,
    weights: &Vec<i128>,
    intercept: i128,
) -> Result<MLPrediction, AutoTradeError> {
    if features.len() != weights.len() {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let mut logit = intercept;
    for i in 0..features.len() {
        let feature = features.get(i).unwrap();
        let weight = weights.get(i).unwrap();
        logit += (feature * weight) / PRECISION;
    }

    let probability = sigmoid(logit);
    let direction = if probability > 5000 {
        TradeDirection::Buy
    } else {
        TradeDirection::Sell
    };

    let confidence = if probability > 5000 {
        probability
    } else {
        10000 - probability
    };

    Ok(MLPrediction {
        direction,
        confidence,
        raw_score: logit,
    })
}

fn sigmoid(x: i128) -> u32 {
    if x >= 6 * PRECISION {
        return 10000;
    }
    if x <= -6 * PRECISION {
        return 0;
    }

    // Piecewise linear approximation around the origin.
    let approximation = 5000 + ((x * 833) / (6 * PRECISION));
    clamp_i128(approximation, 0, 10000) as u32
}

fn predict_decision_tree(
    features: &Vec<i128>,
    nodes: &Vec<TreeNode>,
) -> Result<MLPrediction, AutoTradeError> {
    if nodes.len() == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let mut current_node_idx: u32 = 0;
    let mut guard_steps: u32 = 0;

    loop {
        guard_steps += 1;
        if guard_steps > nodes.len() as u32 + 1 {
            return Err(AutoTradeError::InvalidStatArbConfig);
        }

        let node = nodes.get(current_node_idx).unwrap();

        if let Some(leaf_value) = node.leaf_value {
            return Ok(MLPrediction {
                direction: if leaf_value > 0 {
                    TradeDirection::Buy
                } else {
                    TradeDirection::Sell
                },
                confidence: clamp_i128(leaf_value.abs(), 0, 10000) as u32,
                raw_score: leaf_value,
            });
        }

        if node.feature_index >= features.len() {
            return Err(AutoTradeError::InvalidPriceData);
        }

        let feature_value = features.get(node.feature_index).unwrap();
        current_node_idx = if feature_value <= node.threshold {
            node.left_child.ok_or(AutoTradeError::InvalidStatArbConfig)?
        } else {
            node.right_child.ok_or(AutoTradeError::InvalidStatArbConfig)?
        };

        if current_node_idx >= nodes.len() as u32 {
            return Err(AutoTradeError::InvalidStatArbConfig);
        }
    }
}

fn predict_random_forest(
    features: &Vec<i128>,
    trees: &Vec<DecisionTree>,
    tree_weights: &Vec<u32>,
) -> Result<MLPrediction, AutoTradeError> {
    if trees.len() == 0 || trees.len() != tree_weights.len() {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let mut weighted_sum = 0i128;
    let mut total_weight = 0u32;

    for i in 0..trees.len() {
        let tree = trees.get(i).unwrap();
        let weight = tree_weights.get(i).unwrap();
        if weight == 0 {
            continue;
        }

        let prediction = predict_decision_tree(features, &tree.nodes)?;
        weighted_sum += prediction.raw_score * weight as i128;
        total_weight += weight;
    }

    if total_weight == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let avg_score = weighted_sum / total_weight as i128;
    Ok(MLPrediction {
        direction: if avg_score > 0 {
            TradeDirection::Buy
        } else {
            TradeDirection::Sell
        },
        confidence: clamp_i128(avg_score.abs(), 0, 10000) as u32,
        raw_score: avg_score,
    })
}

fn predict_neural_network(
    features: &Vec<i128>,
    layers: &Vec<Layer>,
) -> Result<MLPrediction, AutoTradeError> {
    if layers.len() == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let mut activations = features.clone();

    for i in 0..layers.len() {
        let layer = layers.get(i).unwrap();
        activations = forward_layer(&activations, &layer)?;
    }

    if activations.len() == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let output = activations.get(0).unwrap();

    Ok(MLPrediction {
        direction: if output > 0 {
            TradeDirection::Buy
        } else {
            TradeDirection::Sell
        },
        confidence: clamp_i128(output.abs(), 0, 10000) as u32,
        raw_score: output,
    })
}

fn forward_layer(inputs: &Vec<i128>, layer: &Layer) -> Result<Vec<i128>, AutoTradeError> {
    if layer.weights.len() != layer.biases.len() {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let mut outputs = Vec::new(&inputs.env());

    for i in 0..layer.weights.len() {
        let neuron_weights = layer.weights.get(i).unwrap();
        let bias = layer.biases.get(i).unwrap();

        if neuron_weights.len() != inputs.len() {
            return Err(AutoTradeError::InvalidPriceData);
        }

        let mut sum = bias;
        for j in 0..inputs.len() {
            let input = inputs.get(j).unwrap();
            let weight = neuron_weights.get(j).unwrap();
            sum += (input * weight) / PRECISION;
        }

        let activated = match layer.activation {
            ActivationFunction::ReLU => {
                if sum > 0 {
                    sum
                } else {
                    0
                }
            }
            ActivationFunction::Sigmoid => sigmoid(sum) as i128,
            ActivationFunction::Tanh => tanh_approx(sum),
            ActivationFunction::Linear => sum,
        };

        outputs.push_back(activated);
    }

    Ok(outputs)
}

fn tanh_approx(x: i128) -> i128 {
    let abs_x = x.abs();
    let denominator = PRECISION + abs_x;
    if denominator == 0 {
        return 0;
    }
    (x * PRECISION) / denominator
}

pub fn check_ml_signal(env: &Env, strategy_id: u64) -> Result<Option<MLSignal>, AutoTradeError> {
    let strategy = get_ml_trading_strategy(env, strategy_id)?;

    if strategy.active_position.position_id != 0 {
        return Ok(None);
    }

    let features = extract_features(env, strategy.asset_pair.clone(), &strategy.feature_config)?;
    let prediction = predict_with_model(&strategy.model, &features)?;

    if prediction.confidence < strategy.prediction_threshold {
        return Ok(None);
    }

    Ok(Some(MLSignal {
        direction: prediction.direction,
        confidence: prediction.confidence,
        features,
        model_version: strategy.model_version,
    }))
}

pub fn execute_ml_trade(
    env: &Env,
    strategy_id: u64,
    signal: MLSignal,
) -> Result<u64, AutoTradeError> {
    let mut strategy = get_ml_trading_strategy(env, strategy_id)?;

    let portfolio_value = get_portfolio_value(strategy.user.clone())?;
    let base_size = (portfolio_value * strategy.position_size_pct as i128) / 10000;
    let confidence_scaled = (base_size * signal.confidence as i128) / 10000;

    let position_id = get_next_position_id(env);
    let current_price = get_current_price(strategy.asset_pair.clone())?;

    strategy.active_position = MLPosition {
        position_id,
        predicted_direction: signal.direction,
        prediction_confidence: signal.confidence,
        entry_price: current_price,
        amount: confidence_scaled,
    };

    set_ml_trading_strategy(env, strategy_id, &strategy);

    Ok(position_id)
}

pub fn update_ml_model(
    env: &Env,
    strategy_id: u64,
    new_model: MLModel,
    new_version: u32,
) -> Result<(), AutoTradeError> {
    let mut strategy = get_ml_trading_strategy(env, strategy_id)?;

    if new_version <= strategy.model_version {
        return Err(AutoTradeError::InvalidAmount);
    }

    validate_model_structure(&new_model)?;

    strategy.model = new_model;
    strategy.model_version = new_version;
    strategy.last_model_update = env.ledger().timestamp();

    set_ml_trading_strategy(env, strategy_id, &strategy);
    Ok(())
}

fn validate_model_structure(model: &MLModel) -> Result<(), AutoTradeError> {
    match model {
        MLModel::LogisticRegression(weights, intercept) => {
            if weights.len() == 0 || weights.len() > MAX_LOGISTIC_WEIGHTS {
                return Err(AutoTradeError::InvalidPriceData);
            }
            if intercept.abs() >= i128::MAX / 2 {
                return Err(AutoTradeError::InvalidPriceData);
            }
        }
        MLModel::DecisionTree(nodes) => {
            if nodes.len() == 0 || nodes.len() > MAX_TREE_NODES {
                return Err(AutoTradeError::InvalidPriceData);
            }
            validate_tree_structure(nodes)?;
        }
        MLModel::RandomForest(trees, tree_weights) => {
            if trees.len() == 0 || trees.len() != tree_weights.len() || trees.len() > MAX_FOREST_TREES {
                return Err(AutoTradeError::InvalidPriceData);
            }
            for i in 0..trees.len() {
                let tree = trees.get(i).unwrap();
                validate_tree_structure(&tree.nodes)?;
            }
        }
        MLModel::NeuralNetwork(layers) => {
            if layers.len() == 0 || layers.len() > MAX_NN_LAYERS {
                return Err(AutoTradeError::InvalidPriceData);
            }
            for i in 0..layers.len() {
                let layer = layers.get(i).unwrap();
                if layer.weights.len() == 0 || layer.weights.len() > MAX_LAYER_NEURONS {
                    return Err(AutoTradeError::InvalidPriceData);
                }
                if layer.weights.len() != layer.biases.len() {
                    return Err(AutoTradeError::InvalidPriceData);
                }
            }
        }
    }

    Ok(())
}

fn validate_tree_structure(nodes: &Vec<TreeNode>) -> Result<(), AutoTradeError> {
    if nodes.len() == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    for i in 0..nodes.len() {
        let node = nodes.get(i).unwrap();

        if node.leaf_value.is_some() {
            continue;
        }

        let left = node.left_child.ok_or(AutoTradeError::InvalidStatArbConfig)?;
        let right = node.right_child.ok_or(AutoTradeError::InvalidStatArbConfig)?;

        if left >= nodes.len() || right >= nodes.len() {
            return Err(AutoTradeError::InvalidStatArbConfig);
        }
    }

    Ok(())
}

pub fn track_ml_performance(
    env: &Env,
    strategy_id: u64,
    position: &MLPosition,
    actual_outcome: bool,
) -> Result<(), AutoTradeError> {
    let mut performance = get_ml_performance(env, strategy_id)?;

    performance.total_predictions += 1;
    let predicted_bullish = position.predicted_direction == TradeDirection::Buy;

    if predicted_bullish == actual_outcome {
        let prev_correct = performance.accurate_predictions;
        performance.accurate_predictions += 1;
        performance.avg_confidence_on_correct = weighted_avg_u32(
            performance.avg_confidence_on_correct,
            prev_correct,
            position.prediction_confidence,
        );
    } else {
        if predicted_bullish {
            performance.false_positives += 1;
        } else {
            performance.false_negatives += 1;
        }

        let incorrect_count = performance.total_predictions - performance.accurate_predictions;
        let prev_incorrect = incorrect_count.saturating_sub(1);
        performance.avg_confidence_on_incorrect = weighted_avg_u32(
            performance.avg_confidence_on_incorrect,
            prev_incorrect,
            position.prediction_confidence,
        );
    }

    set_ml_performance(env, strategy_id, &performance);
    Ok(())
}

pub fn detect_model_drift(env: &Env, strategy_id: u64) -> Result<bool, AutoTradeError> {
    let performance = get_ml_performance(env, strategy_id)?;
    let recent_predictions = get_recent_ml_predictions(env, strategy_id, 50)?;

    if recent_predictions.len() < 20 || performance.total_predictions == 0 {
        return Ok(false);
    }

    let mut correct: u32 = 0;
    for i in 0..recent_predictions.len() {
        let item = recent_predictions.get(i).unwrap();
        if item.was_correct {
            correct += 1;
        }
    }

    let recent_accuracy = (correct * 10000) / recent_predictions.len();
    let historical_accuracy = (performance.accurate_predictions * 10000) / performance.total_predictions;
    let drift_threshold = (historical_accuracy * 80) / 100;

    Ok(recent_accuracy < drift_threshold)
}

fn clamp_i128(value: i128, min: i128, max: i128) -> i128 {
    if value < min {
        return min;
    }
    if value > max {
        return max;
    }
    value
}

fn weighted_avg_u32(prev_avg: u32, prev_count: u32, new_value: u32) -> u32 {
    let total = (prev_avg as u128 * prev_count as u128) + new_value as u128;
    let denom = prev_count as u128 + 1;
    (total / denom) as u32
}

#[allow(unused_variables)]
fn get_ml_trading_strategy(env: &Env, strategy_id: u64) -> Result<MLTradingStrategy, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&MLDataKey::Strategy(strategy_id))
        .ok_or(AutoTradeError::StrategyNotFound)
}

fn set_ml_trading_strategy(env: &Env, strategy_id: u64, strategy: &MLTradingStrategy) {
    env.storage()
        .persistent()
        .set(&MLDataKey::Strategy(strategy_id), strategy);
}

fn get_next_position_id(env: &Env) -> u64 {
    let current = env
        .storage()
        .persistent()
        .get(&MLDataKey::PositionCounter)
        .unwrap_or(0u64);
    let next = current + 1;
    env.storage()
        .persistent()
        .set(&MLDataKey::PositionCounter, &next);
    next
}

fn get_ml_performance(env: &Env, strategy_id: u64) -> Result<MLModelPerformance, AutoTradeError> {
    if let Some(perf) = env
        .storage()
        .persistent()
        .get::<MLDataKey, MLModelPerformance>(&MLDataKey::Performance(strategy_id))
    {
        Ok(perf)
    } else {
        let strategy = get_ml_trading_strategy(env, strategy_id)?;
        Ok(MLModelPerformance {
            model_version: strategy.model_version,
            total_predictions: 0,
            accurate_predictions: 0,
            false_positives: 0,
            false_negatives: 0,
            avg_confidence_on_correct: 0,
            avg_confidence_on_incorrect: 0,
            sharpe_ratio: 0,
        })
    }
}

fn set_ml_performance(env: &Env, strategy_id: u64, performance: &MLModelPerformance) {
    env.storage()
        .persistent()
        .set(&MLDataKey::Performance(strategy_id), performance);
}

#[allow(unused_variables)]
fn get_recent_ml_predictions(
    env: &Env,
    strategy_id: u64,
    limit: u32,
) -> Result<Vec<PredictionRecord>, AutoTradeError> {
    Ok(Vec::new(env))
}

#[allow(unused_variables)]
fn get_current_price(asset_pair: AssetPair) -> Result<i128, AutoTradeError> {
    Ok(100_000)
}

#[allow(unused_variables)]
fn get_24h_volume(asset_pair: AssetPair) -> Result<i128, AutoTradeError> {
    Ok(500_000)
}

#[allow(unused_variables)]
fn calculate_rsi(asset_pair: AssetPair, period: u32) -> Result<u32, AutoTradeError> {
    Ok(5_500)
}

#[allow(unused_variables)]
fn calculate_macd(asset_pair: AssetPair) -> Result<(i128, i128), AutoTradeError> {
    Ok((200, 150))
}

#[allow(unused_variables)]
fn calculate_bollinger_bands(
    asset_pair: AssetPair,
    period: u32,
    stddev: u32,
) -> Result<(i128, i128, i128), AutoTradeError> {
    Ok((110_000, 90_000, 100_000))
}

#[allow(unused_variables)]
fn get_price_n_periods_ago(asset_pair: AssetPair, periods: u32) -> Result<i128, AutoTradeError> {
    Ok(95_000)
}

#[allow(unused_variables)]
fn get_current_volume(asset_pair: AssetPair) -> Result<i128, AutoTradeError> {
    Ok(10_000)
}

#[allow(unused_variables)]
fn get_volume_n_periods_ago(asset_pair: AssetPair, periods: u32) -> Result<i128, AutoTradeError> {
    Ok(9_000)
}

#[allow(unused_variables)]
fn calculate_custom_feature(
    asset_pair: AssetPair,
    name: String,
    calculation: String,
) -> Result<i128, AutoTradeError> {
    Ok(0)
}

#[allow(unused_variables)]
fn get_portfolio_value(user: Address) -> Result<i128, AutoTradeError> {
    Ok(1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{map, vec};

    #[test]
    fn logistic_regression_matches_expected_direction() {
        let env = Env::default();

        let model = MLModel::LogisticRegression(vec![&env, 4000, -2000, 3000], 500);

        let features = vec![&env, 6000, 2000, 3000];
        let prediction = predict_with_model(&model, &features).unwrap();

        assert_eq!(prediction.direction, TradeDirection::Buy);
        assert!(prediction.confidence >= 5000);
    }

    #[test]
    fn decision_tree_traversal_predicts_leaf_value() {
        let env = Env::default();

        // root: feature[0] <= 5000 -> left leaf(+8000), else right leaf(-7000)
        let nodes = vec![
            &env,
            TreeNode {
                feature_index: 0,
                threshold: 5000,
                left_child: Some(1),
                right_child: Some(2),
                leaf_value: None,
            },
            TreeNode {
                feature_index: 0,
                threshold: 0,
                left_child: None,
                right_child: None,
                leaf_value: Some(8000),
            },
            TreeNode {
                feature_index: 0,
                threshold: 0,
                left_child: None,
                right_child: None,
                leaf_value: Some(-7000),
            }
        ];

        let model = MLModel::DecisionTree(nodes);

        let buy_features = vec![&env, 4000];
        let buy_pred = predict_with_model(&model, &buy_features).unwrap();
        assert_eq!(buy_pred.direction, TradeDirection::Buy);
        assert_eq!(buy_pred.confidence, 8000);

        let sell_features = vec![&env, 9000];
        let sell_pred = predict_with_model(&model, &sell_features).unwrap();
        assert_eq!(sell_pred.direction, TradeDirection::Sell);
        assert_eq!(sell_pred.confidence, 7000);
    }

    #[test]
    fn random_forest_aggregates_weighted_scores() {
        let env = Env::default();

        let tree_buy = DecisionTree {
            nodes: vec![
                &env,
                TreeNode {
                    feature_index: 0,
                    threshold: 0,
                    left_child: None,
                    right_child: None,
                    leaf_value: Some(6000),
                }
            ],
        };

        let tree_sell = DecisionTree {
            nodes: vec![
                &env,
                TreeNode {
                    feature_index: 0,
                    threshold: 0,
                    left_child: None,
                    right_child: None,
                    leaf_value: Some(-2000),
                }
            ],
        };

        let model = MLModel::RandomForest(
            vec![&env, tree_buy, tree_sell],
            vec![&env, 3, 1],
        );

        let features = vec![&env, 1000];
        let prediction = predict_with_model(&model, &features).unwrap();
        // (6000*3 + -2000*1)/4 = 4000
        assert_eq!(prediction.raw_score, 4000);
        assert_eq!(prediction.direction, TradeDirection::Buy);
        assert_eq!(prediction.confidence, 4000);
    }

    #[test]
    fn sigmoid_bounds_are_saturated() {
        assert_eq!(sigmoid(10 * PRECISION), 10000);
        assert_eq!(sigmoid(-10 * PRECISION), 0);
    }

    #[test]
    fn normalize_feature_applies_min_max_scaling() {
        let env = Env::default();

        let config = FeatureConfig {
            features: vec![&env, FeatureType::Price],
            normalization: map![
                &env,
                String::from_str(&env, "price") => NormalizationParams { min: 0, max: 200 }
            ],
        };

        let normalized = normalize_feature(&env, 50, &FeatureType::Price, &config).unwrap();
        assert_eq!(normalized, 2500);
    }
}
