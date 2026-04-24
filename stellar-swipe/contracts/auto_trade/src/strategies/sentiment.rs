//! Sentiment-Based Trading with Social Signals
//!
//! Integrates sentiment analysis from multiple sources (social media, news, on-chain metrics)
//! into trading decisions with confidence-weighted position sizing.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Map, String, Symbol, Vec};

/// Sentiment source types
#[contracttype]
#[derive(Clone, Debug)]
pub enum SentimentSource {
    Twitter(String),
    Reddit(String),
    OnChainMetrics(MetricType),
    NewsFeeds(String),
    SignalRationale,
}

/// On-chain metric types
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricType {
    ActiveAddresses,
    TransactionVolume,
    HolderConcentration,
    ExchangeInflows,
}

/// Trade direction
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeDirection {
    Buy,
    Sell,
}

/// Asset pair
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPair {
    pub base: String,
    pub quote: String,
}

/// Sentiment strategy configuration
#[contracttype]
#[derive(Clone, Debug)]
pub struct SentimentStrategy {
    pub strategy_id: u64,
    pub user: Address,
    pub asset_pair: AssetPair,
    pub sentiment_sources: Vec<SentimentSource>,
    pub sentiment_threshold: i32,
    pub tech_confirmation_required: bool,
    pub position_size_pct: u32,
    pub sentiment_decay_hours: u32,
    /// `position_id == 0` means no active position.
    pub active_position: SentimentPosition,
    pub created_at: u64,
}

fn sentiment_position_absent() -> SentimentPosition {
    SentimentPosition {
        position_id: 0,
        entry_sentiment: 0,
        entry_price: 0,
        amount: 0,
        entry_time: 0,
    }
}

/// Sentiment score aggregation
#[contracttype]
#[derive(Clone, Debug)]
pub struct SentimentScore {
    pub overall_score: i32,
    pub confidence: u32,
    pub source_scores: Map<String, i32>,
    pub aggregated_at: u64,
    pub decay_factor: u32,
}

/// Active sentiment position
#[contracttype]
#[derive(Clone, Debug)]
pub struct SentimentPosition {
    pub position_id: u64,
    pub entry_sentiment: i32,
    pub entry_price: i128,
    pub amount: i128,
    pub entry_time: u64,
}

/// Sentiment trading signal
#[contracttype]
#[derive(Clone, Debug)]
pub struct SentimentSignal {
    pub direction: TradeDirection,
    pub sentiment_score: i32,
    pub confidence: u32,
    pub source_breakdown: Map<String, i32>,
}

/// Sentiment accuracy tracking
#[contracttype]
#[derive(Clone, Debug)]
pub struct SentimentAccuracy {
    pub strategy_id: u64,
    pub total_signals: u32,
    pub accurate_predictions: u32,
    pub false_positives: u32,
    pub false_negatives: u32,
    pub avg_sentiment_price_corr: i32,
}

/// Storage keys
#[contracttype]
pub enum SentimentStorageKey {
    StrategyCounter,
    Strategy(u64),
    PositionCounter,
    Accuracy(u64),
    LastSentiment(u64),
}

// Constants
const MIN_SENTIMENT_THRESHOLD: i32 = 1000;
const MAX_SENTIMENT_THRESHOLD: i32 = 9000;
const SCALE_FACTOR: i32 = 10000;
const MAX_POSITION_SIZE_PCT: u32 = 5000; // 50%

/// ==========================
/// Strategy Creation
/// ==========================

/// Create a new sentiment-based trading strategy
pub fn create_sentiment_strategy(
    env: &Env,
    user: Address,
    asset_pair: AssetPair,
    sentiment_sources: Vec<SentimentSource>,
    sentiment_threshold: i32,
    tech_confirmation_required: bool,
    position_size_pct: u32,
    sentiment_decay_hours: u32,
) -> Result<u64, String> {
    user.require_auth();
    
    // Validate inputs
    if sentiment_sources.is_empty() {
        return Err(String::from_str(env, "No sentiment sources"));
    }
    
    if sentiment_threshold < MIN_SENTIMENT_THRESHOLD || sentiment_threshold > MAX_SENTIMENT_THRESHOLD {
        return Err(String::from_str(env, "Invalid sentiment threshold"));
    }
    
    if position_size_pct == 0 || position_size_pct > MAX_POSITION_SIZE_PCT {
        return Err(String::from_str(env, "Invalid position size"));
    }
    
    if sentiment_decay_hours == 0 {
        return Err(String::from_str(env, "Invalid decay hours"));
    }
    
    let strategy_id = get_next_strategy_id(env);
    
    let strategy = SentimentStrategy {
        strategy_id,
        user: user.clone(),
        asset_pair: asset_pair.clone(),
        sentiment_sources,
        sentiment_threshold,
        tech_confirmation_required,
        position_size_pct,
        sentiment_decay_hours,
        active_position: sentiment_position_absent(),
        created_at: env.ledger().timestamp(),
    };
    
    store_strategy(env, strategy_id, &strategy);
    
    // Initialize accuracy tracking
    let accuracy = SentimentAccuracy {
        strategy_id,
        total_signals: 0,
        accurate_predictions: 0,
        false_positives: 0,
        false_negatives: 0,
        avg_sentiment_price_corr: 0,
    };
    store_accuracy(env, strategy_id, &accuracy);
    
    env.events().publish(
        (Symbol::new(env, "sentiment_strategy_created"), strategy_id),
        user,
    );
    
    Ok(strategy_id)
}

/// ==========================
/// Sentiment Aggregation
/// ==========================

/// Aggregate sentiment from all configured sources
pub fn aggregate_sentiment(env: &Env, strategy_id: u64) -> Result<SentimentScore, String> {
    let strategy = get_strategy(env, strategy_id)?;
    
    let mut source_scores = Map::new(env);
    let mut total_weight = 0u32;
    let mut weighted_sum = 0i32;
    
    // Collect sentiment from each source
    for source in strategy.sentiment_sources.iter() {
        let (score, weight) = match source {
            SentimentSource::Twitter(ref handle) => {
                collect_twitter_sentiment(env, handle)?
            }
            SentimentSource::Reddit(ref subreddit) => {
                collect_reddit_sentiment(env, subreddit)?
            }
            SentimentSource::OnChainMetrics(metric_type) => {
                collect_onchain_sentiment(env, &strategy.asset_pair, metric_type)?
            }
            SentimentSource::NewsFeeds(ref feed_url) => {
                collect_news_sentiment(env, feed_url)?
            }
            SentimentSource::SignalRationale => {
                collect_signal_sentiment(env, &strategy.asset_pair)?
            }
        };

        let source_name = format_source_name(env, &source);
        source_scores.set(source_name, score);
        weighted_sum += score * weight as i32;
        total_weight += weight;
    }
    
    let overall_score = if total_weight > 0 {
        weighted_sum / total_weight as i32
    } else {
        0
    };
    
    // Calculate confidence based on agreement between sources
    let confidence = calculate_sentiment_confidence(env, &source_scores)?;
    
    Ok(SentimentScore {
        overall_score,
        confidence,
        source_scores,
        aggregated_at: env.ledger().timestamp(),
        decay_factor: SCALE_FACTOR as u32,
    })
}

/// Calculate confidence based on source agreement
fn calculate_sentiment_confidence(
    env: &Env,
    source_scores: &Map<String, i32>,
) -> Result<u32, String> {
    if source_scores.len() < 2 {
        return Ok(5000); // 50% confidence with single source
    }
    
    // Calculate mean
    let mut sum = 0i32;
    let count = source_scores.len();
    
    for score in source_scores.values() {
        sum += score;
    }
    let mean = sum / count as i32;
    
    // Calculate variance
    let mut variance_sum = 0i32;
    for score in source_scores.values() {
        let diff = score - mean;
        variance_sum += (diff * diff) / count as i32;
    }
    
    // Lower variance = higher confidence
    let std_dev = sqrt(variance_sum as u32);
    let confidence = if std_dev == 0 {
        10000
    } else {
        max(0, 10000 - (std_dev as i32 / 10))
    };
    
    Ok(confidence as u32)
}

fn format_source_name(env: &Env, source: &SentimentSource) -> String {
    match source {
        SentimentSource::Twitter(..) => String::from_str(env, "twitter"),
        SentimentSource::Reddit(..) => String::from_str(env, "reddit"),
        SentimentSource::OnChainMetrics(..) => String::from_str(env, "onchain"),
        SentimentSource::NewsFeeds(..) => String::from_str(env, "news"),
        SentimentSource::SignalRationale => String::from_str(env, "signals"),
    }
}

/// ==========================
/// Sentiment Collection (Placeholders for Oracle Integration)
/// ==========================

/// Collect Twitter sentiment (would integrate with oracle)
fn collect_twitter_sentiment(env: &Env, _handle: &String) -> Result<(i32, u32), String> {
    // Placeholder - would verify oracle-provided sentiment data
    // Oracle analyzes tweets mentioning the asset
    let sentiment_score = 5000; // Neutral for now
    let weight = 30; // Twitter gets 30% weight
    Ok((sentiment_score, weight))
}

/// Collect Reddit sentiment (would integrate with oracle)
fn collect_reddit_sentiment(env: &Env, _subreddit: &String) -> Result<(i32, u32), String> {
    // Placeholder - would verify oracle-provided sentiment data
    let sentiment_score = 6000; // Slightly bullish
    let weight = 25; // Reddit gets 25% weight
    Ok((sentiment_score, weight))
}

/// Collect news sentiment (would integrate with oracle)
fn collect_news_sentiment(env: &Env, _feed_url: &String) -> Result<(i32, u32), String> {
    // Placeholder - would verify oracle-provided sentiment data
    let sentiment_score = 5500;
    let weight = 10; // News gets 10% weight
    Ok((sentiment_score, weight))
}

/// Collect on-chain metrics sentiment
fn collect_onchain_sentiment(
    env: &Env,
    asset_pair: &AssetPair,
    metric_type: MetricType,
) -> Result<(i32, u32), String> {
    let score = match metric_type {
        MetricType::ActiveAddresses => {
            calculate_active_addresses_sentiment(env, asset_pair)?
        }
        MetricType::TransactionVolume => {
            calculate_transaction_volume_sentiment(env, asset_pair)?
        }
        MetricType::HolderConcentration => {
            calculate_holder_concentration_sentiment(env, asset_pair)?
        }
        MetricType::ExchangeInflows => {
            calculate_exchange_inflows_sentiment(env, asset_pair)?
        }
    };
    
    let weight = 35; // On-chain metrics get highest weight (35%)
    Ok((score, weight))
}

/// Calculate sentiment from active addresses
fn calculate_active_addresses_sentiment(
    env: &Env,
    _asset_pair: &AssetPair,
) -> Result<i32, String> {
    // Placeholder - would query actual on-chain data
    // Increasing active addresses = bullish
    let current_addresses = 10000u32;
    let historical_avg = 8000u32;
    
    let change_pct = ((current_addresses as i32 - historical_avg as i32) * 100) / historical_avg as i32;
    let sentiment = change_pct * 100;
    
    Ok(clamp(sentiment, -SCALE_FACTOR, SCALE_FACTOR))
}

/// Calculate sentiment from transaction volume
fn calculate_transaction_volume_sentiment(
    env: &Env,
    _asset_pair: &AssetPair,
) -> Result<i32, String> {
    // Placeholder - increasing volume = bullish
    let current_volume = 1_000_000i128;
    let historical_avg = 800_000i128;
    
    let change_pct = ((current_volume - historical_avg) * 100) / historical_avg;
    let sentiment = (change_pct * 100) as i32;
    
    Ok(clamp(sentiment, -SCALE_FACTOR, SCALE_FACTOR))
}

/// Calculate sentiment from holder concentration
fn calculate_holder_concentration_sentiment(
    env: &Env,
    _asset_pair: &AssetPair,
) -> Result<i32, String> {
    // Placeholder - decreasing concentration = bullish (more distribution)
    let top_10_pct = 4000u32; // 40% held by top 10
    let ideal_pct = 3000u32;  // 30% is ideal
    
    let diff = ideal_pct as i32 - top_10_pct as i32;
    let sentiment = diff * 100;
    
    Ok(clamp(sentiment, -SCALE_FACTOR, SCALE_FACTOR))
}

/// Calculate sentiment from exchange inflows
fn calculate_exchange_inflows_sentiment(
    env: &Env,
    _asset_pair: &AssetPair,
) -> Result<i32, String> {
    // High exchange inflows = bearish (potential selling)
    // High exchange outflows = bullish (accumulation)
    let inflows_24h = 500_000i128;
    let outflows_24h = 700_000i128;
    
    let net_flow = outflows_24h - inflows_24h;
    let total_supply = 10_000_000i128;
    
    let net_flow_pct = (net_flow * SCALE_FACTOR as i128) / total_supply;
    
    Ok((net_flow_pct * 100) as i32)
}

/// Collect sentiment from signal rationales
fn collect_signal_sentiment(env: &Env, _asset_pair: &AssetPair) -> Result<(i32, u32), String> {
    // Placeholder - would analyze recent signal rationales
    let sentiment_score = analyze_rationale_sentiment(
        env,
        &String::from_str(env, "Bullish breakout expected with strong momentum")
    )?;
    
    let weight = 10; // 10% weight for signal sentiment
    Ok((sentiment_score, weight))
}

/// Analyze sentiment from text rationale
fn analyze_rationale_sentiment(env: &Env, rationale: &String) -> Result<i32, String> {
    let n = rationale.len() as usize;
    if n > 256 {
        return Ok(0);
    }
    let mut buf = [0u8; 256];
    rationale.copy_into_slice(&mut buf[..n]);
    for b in &mut buf[..n] {
        if *b >= b'A' && *b <= b'Z' {
            *b += 32;
        }
    }
    let text = core::str::from_utf8(&buf[..n])
        .map_err(|_| String::from_str(env, "invalid utf8"))?;
    
    // Bullish keywords
    let bullish_keywords = ["bullish", "buy", "breakout", "moon", "pump", "strong", 
                           "uptrend", "accumulate", "undervalued", "rally"];
    
    // Bearish keywords
    let bearish_keywords = ["bearish", "sell", "dump", "crash", "weak", "downtrend",
                           "overvalued", "resistance", "distribution", "decline"];
    
    let mut bullish_count = 0i32;
    let mut bearish_count = 0i32;
    
    for keyword in bullish_keywords.iter() {
        if text.contains(keyword) {
            bullish_count += 1;
        }
    }
    
    for keyword in bearish_keywords.iter() {
        if text.contains(keyword) {
            bearish_count += 1;
        }
    }
    
    let net_sentiment = (bullish_count - bearish_count) * 2000;
    Ok(clamp(net_sentiment, -SCALE_FACTOR, SCALE_FACTOR))
}

/// ==========================
/// Sentiment Decay
/// ==========================

/// Apply time-based decay to sentiment score
pub fn apply_sentiment_decay(
    env: &Env,
    sentiment_score: &mut SentimentScore,
    decay_hours: u32,
) -> Result<(), String> {
    let elapsed_seconds = env.ledger().timestamp() - sentiment_score.aggregated_at;
    let elapsed_hours = elapsed_seconds / 3600;
    
    if elapsed_hours > 0 {
        // Linear decay over specified hours
        let decay_pct = min(SCALE_FACTOR as u64, (elapsed_hours * SCALE_FACTOR as u64) / decay_hours as u64);
        sentiment_score.decay_factor = (SCALE_FACTOR as u64 - decay_pct) as u32;
        
        // Apply decay to overall score
        sentiment_score.overall_score = (sentiment_score.overall_score * sentiment_score.decay_factor as i32) / SCALE_FACTOR;
    }
    
    Ok(())
}

/// ==========================
/// Signal Generation
/// ==========================

/// Check if sentiment generates a trading signal
pub fn check_sentiment_signal(
    env: &Env,
    strategy_id: u64,
) -> Result<Option<SentimentSignal>, String> {
    let strategy = get_strategy(env, strategy_id)?;
    
    // Don't open new position if one exists
    if strategy.active_position.position_id != 0 {
        return Ok(None);
    }
    
    let mut sentiment = aggregate_sentiment(env, strategy_id)?;
    
    // Apply decay
    apply_sentiment_decay(env, &mut sentiment, strategy.sentiment_decay_hours)?;
    
    // Store latest sentiment
    store_last_sentiment(env, strategy_id, &sentiment);
    
    // Check if sentiment exceeds threshold
    if sentiment.overall_score.abs() < strategy.sentiment_threshold {
        return Ok(None);
    }
    
    // Check technical confirmation if required
    if strategy.tech_confirmation_required {
        let technical_confirmed = check_technical_confirmation(
            env,
            &strategy.asset_pair,
            sentiment.overall_score > 0,
        )?;
        
        if !technical_confirmed {
            return Ok(None);
        }
    }
    
    let signal = SentimentSignal {
        direction: if sentiment.overall_score > 0 {
            TradeDirection::Buy
        } else {
            TradeDirection::Sell
        },
        sentiment_score: sentiment.overall_score,
        confidence: sentiment.confidence,
        source_breakdown: sentiment.source_scores,
    };
    
    Ok(Some(signal))
}

/// Check technical confirmation (placeholder)
fn check_technical_confirmation(
    env: &Env,
    _asset_pair: &AssetPair,
    is_bullish: bool,
) -> Result<bool, String> {
    // Placeholder - would calculate actual technical indicators
    // RSI, MACD, etc.
    let rsi = 5500u32; // Slightly above 50
    let macd_bullish = true;
    
    if is_bullish {
        Ok(rsi > 5000 && macd_bullish)
    } else {
        Ok(rsi < 5000 && !macd_bullish)
    }
}

/// ==========================
/// Trade Execution
/// ==========================

/// Execute sentiment-based trade
pub fn execute_sentiment_trade(
    env: &Env,
    strategy_id: u64,
    signal: SentimentSignal,
) -> Result<u64, String> {
    let mut strategy = get_strategy(env, strategy_id)?;
    
    // Get portfolio value (placeholder)
    let portfolio_value = 1_000_000i128;
    
    // Adjust position size based on sentiment confidence
    let confidence_multiplier = signal.confidence as i128;
    let base_size = (portfolio_value * strategy.position_size_pct as i128) / SCALE_FACTOR as i128;
    let position_amount = (base_size * confidence_multiplier) / SCALE_FACTOR as i128;
    
    // Get current price (placeholder)
    let current_price = 100_000i128;
    
    // Create position
    let position_id = get_next_position_id(env);
    
    let position = SentimentPosition {
        position_id,
        entry_sentiment: signal.sentiment_score,
        entry_price: current_price,
        amount: position_amount,
        entry_time: env.ledger().timestamp(),
    };
    
    strategy.active_position = position;
    store_strategy(env, strategy_id, &strategy);
    
    env.events().publish(
        (Symbol::new(env, "sentiment_trade_executed"), strategy_id, position_id),
        (signal.sentiment_score, signal.confidence),
    );
    
    Ok(position_id)
}

/// ==========================
/// Position Monitoring & Exit
/// ==========================

/// Check if position should be exited
pub fn check_sentiment_exit(
    env: &Env,
    strategy_id: u64,
) -> Result<Option<u64>, String> {
    let mut strategy = get_strategy(env, strategy_id)?;
    
    if strategy.active_position.position_id == 0 {
        return Ok(None);
    }
    let position = strategy.active_position.clone();
    
    // Get current sentiment
    let mut current_sentiment = aggregate_sentiment(env, strategy_id)?;
    apply_sentiment_decay(env, &mut current_sentiment, strategy.sentiment_decay_hours)?;
    
    // Exit conditions
    let sentiment_reversed = 
        (position.entry_sentiment > 0 && current_sentiment.overall_score < -strategy.sentiment_threshold) ||
        (position.entry_sentiment < 0 && current_sentiment.overall_score > strategy.sentiment_threshold);
    
    let sentiment_weakened = current_sentiment.overall_score.abs() < 1000; // Near neutral
    
    // Check profit/loss
    let current_price = 105_000i128; // Placeholder
    let pnl_pct = ((current_price - position.entry_price) * 100) / position.entry_price;
    let profit_target_hit = pnl_pct > 10; // 10% profit
    let stop_loss_hit = pnl_pct < -5; // -5% loss
    
    if sentiment_reversed || sentiment_weakened || profit_target_hit || stop_loss_hit {
        let reason = if sentiment_reversed {
            "Sentiment reversed"
        } else if sentiment_weakened {
            "Sentiment weakened"
        } else if profit_target_hit {
            "Profit target"
        } else {
            "Stop loss"
        };
        
        // Track accuracy
        track_sentiment_accuracy(env, strategy_id, &position, current_price)?;
        
        env.events().publish(
            (Symbol::new(env, "sentiment_position_closed"), strategy_id, position.position_id),
            (current_sentiment.overall_score, String::from_str(env, reason)),
        );
        
        let position_id = position.position_id;
        strategy.active_position = sentiment_position_absent();
        store_strategy(env, strategy_id, &strategy);
        
        return Ok(Some(position_id));
    }
    
    Ok(None)
}

/// ==========================
/// Accuracy Tracking
/// ==========================

/// Track sentiment prediction accuracy
fn track_sentiment_accuracy(
    env: &Env,
    strategy_id: u64,
    position: &SentimentPosition,
    exit_price: i128,
) -> Result<(), String> {
    let mut accuracy = get_accuracy(env, strategy_id)?;
    
    let pnl = exit_price - position.entry_price;
    let predicted_direction = position.entry_sentiment > 0;
    let actual_direction = pnl > 0;
    
    accuracy.total_signals += 1;
    
    if predicted_direction == actual_direction {
        accuracy.accurate_predictions += 1;
    } else if predicted_direction && !actual_direction {
        accuracy.false_positives += 1;
    } else {
        accuracy.false_negatives += 1;
    }
    
    // Update rolling correlation (simplified)
    let correlation = if accuracy.total_signals > 0 {
        ((accuracy.accurate_predictions * SCALE_FACTOR as u32) / accuracy.total_signals) as i32
    } else {
        0
    };
    accuracy.avg_sentiment_price_corr = correlation;
    
    store_accuracy(env, strategy_id, &accuracy);
    
    Ok(())
}

/// Get sentiment accuracy stats
pub fn get_sentiment_accuracy(env: &Env, strategy_id: u64) -> Result<SentimentAccuracy, String> {
    get_accuracy(env, strategy_id)
}

/// ==========================
/// Storage Functions
/// ==========================

fn get_next_strategy_id(env: &Env) -> u64 {
    let counter: u64 = env
        .storage()
        .persistent()
        .get(&SentimentStorageKey::StrategyCounter)
        .unwrap_or(0);
    let next_id = counter + 1;
    env.storage()
        .persistent()
        .set(&SentimentStorageKey::StrategyCounter, &next_id);
    next_id
}

fn get_next_position_id(env: &Env) -> u64 {
    let counter: u64 = env
        .storage()
        .persistent()
        .get(&SentimentStorageKey::PositionCounter)
        .unwrap_or(0);
    let next_id = counter + 1;
    env.storage()
        .persistent()
        .set(&SentimentStorageKey::PositionCounter, &next_id);
    next_id
}

fn store_strategy(env: &Env, strategy_id: u64, strategy: &SentimentStrategy) {
    env.storage()
        .persistent()
        .set(&SentimentStorageKey::Strategy(strategy_id), strategy);
}

fn get_strategy(env: &Env, strategy_id: u64) -> Result<SentimentStrategy, String> {
    env.storage()
        .persistent()
        .get(&SentimentStorageKey::Strategy(strategy_id))
        .ok_or_else(|| String::from_str(env, "Strategy not found"))
}

fn store_accuracy(env: &Env, strategy_id: u64, accuracy: &SentimentAccuracy) {
    env.storage()
        .persistent()
        .set(&SentimentStorageKey::Accuracy(strategy_id), accuracy);
}

fn get_accuracy(env: &Env, strategy_id: u64) -> Result<SentimentAccuracy, String> {
    env.storage()
        .persistent()
        .get(&SentimentStorageKey::Accuracy(strategy_id))
        .ok_or_else(|| String::from_str(env, "Accuracy not found"))
}

fn store_last_sentiment(env: &Env, strategy_id: u64, sentiment: &SentimentScore) {
    env.storage()
        .persistent()
        .set(&SentimentStorageKey::LastSentiment(strategy_id), sentiment);
}

pub fn get_last_sentiment(env: &Env, strategy_id: u64) -> Option<SentimentScore> {
    env.storage()
        .persistent()
        .get(&SentimentStorageKey::LastSentiment(strategy_id))
}

/// ==========================
/// Utility Functions
/// ==========================

fn clamp(value: i32, min_val: i32, max_val: i32) -> i32 {
    if value < min_val {
        min_val
    } else if value > max_val {
        max_val
    } else {
        value
    }
}

fn max(a: i32, b: i32) -> i32 {
    if a > b { a } else { b }
}

fn min(a: u64, b: u64) -> u64 {
    if a < b { a } else { b }
}

fn sqrt(n: u32) -> u32 {
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

/// ==========================
/// Tests
/// ==========================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger as _}, Env};

    fn setup_env() -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(1000);
        env
    }

    fn create_test_asset_pair(env: &Env) -> AssetPair {
        AssetPair {
            base: String::from_str(env, "XLM"),
            quote: String::from_str(env, "USDC"),
        }
    }

    fn create_test_sources(env: &Env) -> Vec<SentimentSource> {
        let mut sources = Vec::new(env);
        sources.push_back(SentimentSource::Twitter(String::from_str(env, "stellar")));
        sources.push_back(SentimentSource::OnChainMetrics(MetricType::ActiveAddresses));
        sources.push_back(SentimentSource::SignalRationale);
        sources
    }

    #[test]
    fn test_create_sentiment_strategy() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        let result = create_sentiment_strategy(
            &env,
            user.clone(),
            asset_pair,
            sources,
            5000,  // 50% threshold
            true,  // require technical confirmation
            2000,  // 20% position size
            24,    // 24 hour decay
        );
        
        assert!(result.is_ok());
        let strategy_id = result.unwrap();
        assert_eq!(strategy_id, 1);
        
        let strategy = get_strategy(&env, strategy_id).unwrap();
        assert_eq!(strategy.user, user);
        assert_eq!(strategy.sentiment_threshold, 5000);
        assert_eq!(strategy.position_size_pct, 2000);
    }

    #[test]
    fn test_create_strategy_invalid_threshold() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        // Too low threshold
        let result = create_sentiment_strategy(
            &env,
            user.clone(),
            asset_pair.clone(),
            sources.clone(),
            500,   // Below minimum
            true,
            2000,
            24,
        );
        assert!(result.is_err());
        
        // Too high threshold
        let result = create_sentiment_strategy(
            &env,
            user,
            asset_pair,
            sources,
            9500,  // Above maximum
            true,
            2000,
            24,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_aggregate_sentiment() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        let strategy_id = create_sentiment_strategy(
            &env,
            user,
            asset_pair,
            sources,
            5000,
            false,
            2000,
            24,
        ).unwrap();
        
        let sentiment = aggregate_sentiment(&env, strategy_id).unwrap();
        
        assert!(sentiment.overall_score >= -SCALE_FACTOR);
        assert!(sentiment.overall_score <= SCALE_FACTOR);
        assert!(sentiment.confidence > 0);
        assert!(sentiment.confidence <= 10000);
        assert!(sentiment.source_scores.len() > 0);
    }

    #[test]
    fn test_sentiment_decay() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        let strategy_id = create_sentiment_strategy(
            &env,
            user,
            asset_pair,
            sources,
            5000,
            false,
            2000,
            24,
        ).unwrap();
        
        let mut sentiment = aggregate_sentiment(&env, strategy_id).unwrap();
        let original_score = sentiment.overall_score;
        
        // Advance time by 12 hours
        env.ledger().set_timestamp(1000 + 12 * 3600);
        
        apply_sentiment_decay(&env, &mut sentiment, 24).unwrap();
        
        // Score should be decayed (50% after 12 hours with 24 hour decay)
        assert!(sentiment.overall_score.abs() < original_score.abs());
        assert_eq!(sentiment.decay_factor, 5000); // 50%
    }

    #[test]
    fn test_check_sentiment_signal_bullish() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        let strategy_id = create_sentiment_strategy(
            &env,
            user,
            asset_pair,
            sources,
            3000,  // Lower threshold to trigger
            false, // No technical confirmation
            2000,
            24,
        ).unwrap();
        
        let signal = check_sentiment_signal(&env, strategy_id).unwrap();
        
        // Should generate signal if sentiment exceeds threshold
        if let Some(sig) = signal {
            assert!(sig.sentiment_score.abs() >= 3000);
            assert!(sig.confidence > 0);
        }
    }

    #[test]
    fn test_execute_sentiment_trade() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        let strategy_id = create_sentiment_strategy(
            &env,
            user,
            asset_pair,
            sources,
            3000,
            false,
            2000,
            24,
        ).unwrap();
        
        let mut source_scores = Map::new(&env);
        source_scores.set(String::from_str(&env, "test"), 7000);
        
        let signal = SentimentSignal {
            direction: TradeDirection::Buy,
            sentiment_score: 7000,
            confidence: 8000,
            source_breakdown: source_scores,
        };
        
        let result = execute_sentiment_trade(&env, strategy_id, signal);
        assert!(result.is_ok());
        
        let position_id = result.unwrap();
        assert_eq!(position_id, 1);
        
        let strategy = get_strategy(&env, strategy_id).unwrap();
        assert_ne!(strategy.active_position.position_id, 0);

        let position = strategy.active_position;
        assert_eq!(position.entry_sentiment, 7000);
    }

    #[test]
    fn test_check_sentiment_exit() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        let strategy_id = create_sentiment_strategy(
            &env,
            user,
            asset_pair,
            sources,
            3000,
            false,
            2000,
            24,
        ).unwrap();
        
        // Create a position
        let mut source_scores = Map::new(&env);
        source_scores.set(String::from_str(&env, "test"), 7000);
        
        let signal = SentimentSignal {
            direction: TradeDirection::Buy,
            sentiment_score: 7000,
            confidence: 8000,
            source_breakdown: source_scores,
        };
        
        execute_sentiment_trade(&env, strategy_id, signal).unwrap();
        
        // Check exit (would exit based on conditions)
        let exit_result = check_sentiment_exit(&env, strategy_id);
        assert!(exit_result.is_ok());
    }

    #[test]
    fn test_sentiment_accuracy_tracking() {
        let env = setup_env();
        let user = Address::generate(&env);
        let asset_pair = create_test_asset_pair(&env);
        let sources = create_test_sources(&env);
        
        env.mock_all_auths();
        
        let strategy_id = create_sentiment_strategy(
            &env,
            user,
            asset_pair,
            sources,
            3000,
            false,
            2000,
            24,
        ).unwrap();
        
        let position = SentimentPosition {
            position_id: 1,
            entry_sentiment: 7000, // Bullish
            entry_price: 100_000,
            amount: 10_000,
            entry_time: 1000,
        };
        
        // Exit at higher price (correct prediction)
        track_sentiment_accuracy(&env, strategy_id, &position, 110_000).unwrap();
        
        let accuracy = get_sentiment_accuracy(&env, strategy_id).unwrap();
        assert_eq!(accuracy.total_signals, 1);
        assert_eq!(accuracy.accurate_predictions, 1);
        assert_eq!(accuracy.false_positives, 0);
    }

    #[test]
    fn test_analyze_rationale_sentiment() {
        let env = setup_env();
        
        let bullish_text = String::from_str(&env, "Very bullish breakout with strong momentum");
        let sentiment = analyze_rationale_sentiment(&env, &bullish_text).unwrap();
        assert!(sentiment > 0);
        
        let bearish_text = String::from_str(&env, "Bearish trend with weak support and potential crash");
        let sentiment = analyze_rationale_sentiment(&env, &bearish_text).unwrap();
        assert!(sentiment < 0);
        
        let neutral_text = String::from_str(&env, "Market analysis shows mixed signals");
        let sentiment = analyze_rationale_sentiment(&env, &neutral_text).unwrap();
        assert_eq!(sentiment, 0);
    }

    #[test]
    fn test_calculate_sentiment_confidence() {
        let env = setup_env();
        
        // High agreement = high confidence
        let mut scores = Map::new(&env);
        scores.set(String::from_str(&env, "source1"), 7000);
        scores.set(String::from_str(&env, "source2"), 7100);
        scores.set(String::from_str(&env, "source3"), 6900);
        
        let confidence = calculate_sentiment_confidence(&env, &scores).unwrap();
        assert!(confidence > 8000); // High confidence
        
        // Low agreement = low confidence
        let mut scores2 = Map::new(&env);
        scores2.set(String::from_str(&env, "source1"), 8000);
        scores2.set(String::from_str(&env, "source2"), 2000);
        scores2.set(String::from_str(&env, "source3"), 5000);
        
        let confidence2 = calculate_sentiment_confidence(&env, &scores2).unwrap();
        assert!(confidence2 < confidence); // Lower confidence
    }

    #[test]
    fn test_clamp() {
        assert_eq!(clamp(5000, 0, 10000), 5000);
        assert_eq!(clamp(-1000, 0, 10000), 0);
        assert_eq!(clamp(15000, 0, 10000), 10000);
    }

    #[test]
    fn test_sqrt() {
        assert_eq!(sqrt(0), 0);
        assert_eq!(sqrt(1), 1);
        assert_eq!(sqrt(4), 2);
        assert_eq!(sqrt(9), 3);
        assert_eq!(sqrt(16), 4);
        assert_eq!(sqrt(100), 10);
    }
}
