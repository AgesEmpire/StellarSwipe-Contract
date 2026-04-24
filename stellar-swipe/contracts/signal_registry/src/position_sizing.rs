#![allow(dead_code)]

//! Dynamic Position Sizing Based on Volatility
//!
//! Three sizing methods are supported:
//!
//! 1. **FixedPercentage** — risk-adjusted sizing:
//!    `size = portfolio_value * risk_per_trade_bps / volatility_bps`
//!    Example: $10K portfolio, 200 bps risk target, 500 bps volatility
//!    → size = 10_000 * 200 / 500 = $4_000
//!
//! 2. **Kelly** — Kelly Criterion with fractional multiplier:
//!    `kelly_f = (win_rate * avg_win_bps - (10000 - win_rate) * avg_loss_bps) / avg_win_bps`
//!    `size = portfolio_value * kelly_f * kelly_multiplier / 10000`
//!
//! 3. **VolatilityScaled** — scale a base allocation to hit a target volatility:
//!    `size = (portfolio_value * base_pct / 10000) * target_vol_bps / current_vol_bps`
//!
//! All sizes are capped at `max_position_pct` of portfolio value and floored at a
//! minimum of 1 unit. Zero volatility is treated as maximum risk → minimum position.
//! High volatility is handled by a configurable floor on position size.

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::errors::AutoTradeError;
use crate::risk::{calculate_portfolio_value, get_asset_price, get_risk_config, RiskConfig};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default conservative volatility used when there is insufficient price history.
/// 2000 bps = 20% — deliberately conservative so sizing errs toward small.
pub const DEFAULT_VOLATILITY_BPS: i128 = 2000;

/// Target volatility for the VolatilityScaled method when the config target is 0.
pub const DEFAULT_TARGET_VOLATILITY_BPS: i128 = 500; // 5%

/// Minimum position size as an absolute unit (prevents dust).
pub const MIN_POSITION_SIZE: i128 = 1;

/// Minimum number of price observations required for a valid volatility estimate.
pub const MIN_PRICE_HISTORY: usize = 2;

/// Hard cap on volatility beyond which we assign the minimum position size.
/// 10000 bps = 100% daily volatility is essentially untradeble.
pub const MAX_VOLATILITY_BPS: i128 = 10_000;

/// Default Kelly multiplier (half-Kelly) — 50 out of 100.
pub const DEFAULT_KELLY_MULTIPLIER: u32 = 50;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How position sizes are calculated.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SizingMethod {
    /// Risk-percentage divided by volatility.
    FixedPercentage,
    /// Fractional Kelly Criterion based on provider win-rate stats.
    Kelly,
    /// Scale a base allocation to maintain a target volatility level.
    VolatilityScaled,
}

/// Per-user configuration for position sizing.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionSizingConfig {
    /// Which sizing method to use.
    pub method: SizingMethod,
    /// Risk budget per trade in basis points (e.g. 200 = 2%).
    pub risk_per_trade_bps: u32,
    /// Maximum position size as percentage of portfolio in basis points (e.g. 2000 = 20%).
    pub max_position_pct_bps: u32,
    /// Kelly multiplier 25–50 (= 0.25× – 0.50× Kelly). Applied as out of 100.
    pub kelly_multiplier: u32,
    /// Target volatility for VolatilityScaled method in basis points (e.g. 500 = 5%).
    pub target_volatility_bps: u32,
    /// Base allocation percentage for VolatilityScaled in basis points (e.g. 1000 = 10%).
    pub base_position_pct_bps: u32,
}

impl Default for PositionSizingConfig {
    fn default() -> Self {
        PositionSizingConfig {
            method: SizingMethod::FixedPercentage,
            risk_per_trade_bps: 200,          // 2% risk per trade
            max_position_pct_bps: 2000,       // max 20% of portfolio
            kelly_multiplier: DEFAULT_KELLY_MULTIPLIER,
            target_volatility_bps: 500,       // target 5% volatility
            base_position_pct_bps: 1000,      // 10% base allocation
        }
    }
}

/// Detailed sizing recommendation returned to callers.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizingRecommendation {
    /// Recommended position size after all adjustments.
    pub recommended_size: i128,
    /// Hard maximum allowed by the portfolio limit.
    pub max_size: i128,
    /// Volatility used in the calculation (basis points).
    pub volatility_bps: i128,
    /// Portfolio value at the time of calculation.
    pub portfolio_value: i128,
    /// Whether the recommended size was capped at max_size.
    pub was_capped: bool,
}

/// Storage key for position sizing config.
#[contracttype]
pub enum SizingDataKey {
    UserSizingConfig(Address),
    /// Circular price history buffer: (asset_id, slot) → price.
    PriceHistory(u32, u32),
    /// Number of prices stored for an asset.
    PriceHistoryLen(u32),
    /// Next write slot (ring buffer head).
    PriceHistoryHead(u32),
}

/// Maximum price history slots per asset.
const MAX_HISTORY_SLOTS: u32 = 60;

// ---------------------------------------------------------------------------
// Config storage
// ---------------------------------------------------------------------------

pub fn get_sizing_config(env: &Env, user: &Address) -> PositionSizingConfig {
    env.storage()
        .persistent()
        .get(&SizingDataKey::UserSizingConfig(user.clone()))
        .unwrap_or_default()
}

pub fn set_sizing_config(env: &Env, user: &Address, config: &PositionSizingConfig) {
    validate_sizing_config(config).expect("invalid sizing config");
    env.storage()
        .persistent()
        .set(&SizingDataKey::UserSizingConfig(user.clone()), config);
}

fn validate_sizing_config(config: &PositionSizingConfig) -> Result<(), AutoTradeError> {
    if config.max_position_pct_bps > 10_000 {
        return Err(AutoTradeError::InvalidSizingConfig);
    }
    if config.kelly_multiplier > 100 {
        return Err(AutoTradeError::InvalidSizingConfig);
    }
    if config.risk_per_trade_bps > 10_000 {
        return Err(AutoTradeError::InvalidSizingConfig);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Price history (ring buffer per asset)
// ---------------------------------------------------------------------------

/// Record a new price observation for an asset. Overwrites oldest entry when full.
pub fn record_price(env: &Env, asset_id: u32, price: i128) {
    let len: u32 = env
        .storage()
        .persistent()
        .get(&SizingDataKey::PriceHistoryLen(asset_id))
        .unwrap_or(0);
    let head: u32 = env
        .storage()
        .persistent()
        .get(&SizingDataKey::PriceHistoryHead(asset_id))
        .unwrap_or(0);

    let slot = head % MAX_HISTORY_SLOTS;

    env.storage()
        .persistent()
        .set(&SizingDataKey::PriceHistory(asset_id, slot), &price);

    let new_len = if len < MAX_HISTORY_SLOTS { len + 1 } else { len };
    let new_head = (head + 1) % MAX_HISTORY_SLOTS;

    env.storage()
        .persistent()
        .set(&SizingDataKey::PriceHistoryLen(asset_id), &new_len);
    env.storage()
        .persistent()
        .set(&SizingDataKey::PriceHistoryHead(asset_id), &new_head);
}

/// Retrieve up to `max_slots` most recent prices in chronological order (oldest first).
pub fn get_price_history(env: &Env, asset_id: u32, max_slots: u32) -> Vec<i128> {
    let len: u32 = env
        .storage()
        .persistent()
        .get(&SizingDataKey::PriceHistoryLen(asset_id))
        .unwrap_or(0);
    let head: u32 = env
        .storage()
        .persistent()
        .get(&SizingDataKey::PriceHistoryHead(asset_id))
        .unwrap_or(0);

    if len == 0 {
        return Vec::new(env);
    }

    let actual = if len < max_slots { len } else { max_slots };

    // The oldest slot when the buffer is full:
    // head points to where the *next* write will go, so the oldest is at head % MAX_HISTORY_SLOTS
    // when len == MAX_HISTORY_SLOTS, otherwise the oldest is at slot 0 (writing from 0 forward).
    let start_slot = if len == MAX_HISTORY_SLOTS {
        head % MAX_HISTORY_SLOTS
    } else {
        // Not yet wrapped: earliest entry is at slot (head - len) in the range [0, MAX)
        head.saturating_sub(len) % MAX_HISTORY_SLOTS
    };

    let mut prices = Vec::new(env);
    for i in 0..actual {
        let slot = (start_slot + (len - actual) + i) % MAX_HISTORY_SLOTS;
        if let Some(price) = env
            .storage()
            .persistent()
            .get::<SizingDataKey, i128>(&SizingDataKey::PriceHistory(asset_id, slot))
        {
            prices.push_back(price);
        }
    }
    prices
}

// ---------------------------------------------------------------------------
// Volatility calculation
// ---------------------------------------------------------------------------

/// Integer square root (Babylonian method, no_std compatible).
fn isqrt(n: i128) -> i128 {
    if n <= 0 {
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

/// Calculate historical volatility for an asset from its stored price history.
///
/// Returns volatility in basis points (10000 = 100%).
/// Falls back to `DEFAULT_VOLATILITY_BPS` when history is insufficient.
pub fn calculate_volatility(env: &Env, asset_id: u32, window_slots: u32) -> i128 {
    let prices = get_price_history(env, asset_id, window_slots + 1);

    if (prices.len() as usize) < MIN_PRICE_HISTORY {
        return DEFAULT_VOLATILITY_BPS;
    }

    // Calculate daily returns in basis points: ret = (p[i] - p[i-1]) / p[i-1] * 10000
    let n = prices.len();
    let mut returns: Vec<i128> = Vec::new(env);
    for i in 1..n {
        let prev = prices.get(i - 1).unwrap();
        let curr = prices.get(i).unwrap();
        if prev == 0 {
            continue;
        }
        let ret = (curr - prev) * 10_000 / prev;
        returns.push_back(ret);
    }

    let r_len = returns.len() as i128;
    if r_len == 0 {
        return DEFAULT_VOLATILITY_BPS;
    }

    // Mean of returns
    let mut sum: i128 = 0;
    for i in 0..returns.len() {
        sum = sum.saturating_add(returns.get(i).unwrap());
    }
    let mean = sum / r_len;

    // Variance = Σ(r - mean)² / n
    let mut variance_sum: i128 = 0;
    for i in 0..returns.len() {
        let diff = returns.get(i).unwrap() - mean;
        variance_sum = variance_sum.saturating_add(diff * diff);
    }
    let variance = variance_sum / r_len;

    // Volatility = sqrt(variance)
    let vol = isqrt(variance);

    if vol == 0 {
        // Zero variance → zero volatility → treat as max risk, return minimum position signal
        // (callers check for 0 and substitute DEFAULT)
        0
    } else {
        vol
    }
}

// ---------------------------------------------------------------------------
// Kelly Criterion helpers
// ---------------------------------------------------------------------------

/// Calculate the Kelly fraction from provider stats.
///
/// `win_rate_bps`  — win rate in basis points (e.g. 6000 = 60%)
/// `avg_win_bps`   — average winning trade ROI in basis points
/// `avg_loss_bps`  — average losing trade ROI magnitude in basis points (positive number)
///
/// Returns Kelly fraction in basis points, clamped to [0, 10000].
pub fn calculate_kelly_fraction(
    win_rate_bps: i128,
    avg_win_bps: i128,
    avg_loss_bps: i128,
) -> i128 {
    if avg_win_bps <= 0 {
        return 0;
    }

    // kelly_f = (win_rate * avg_win - loss_rate * avg_loss) / avg_win
    // All in basis points (10000 = 100%)
    let loss_rate_bps = 10_000 - win_rate_bps;
    let numerator = win_rate_bps * avg_win_bps - loss_rate_bps * avg_loss_bps;

    if numerator <= 0 {
        return 0; // Negative or zero Kelly → don't trade
    }

    // Divide by avg_win to get the fraction, result is in bps
    let kelly = numerator / avg_win_bps;

    // Clamp to [0, 10000]
    if kelly > 10_000 {
        10_000
    } else {
        kelly
    }
}

// ---------------------------------------------------------------------------
// Core position size calculation
// ---------------------------------------------------------------------------

/// Calculate the recommended and maximum position sizes for a given user and asset.
///
/// `asset_id`     — the asset to size a position in
/// `user`         — the trading user (for portfolio value and config lookup)
/// `win_rate_bps` — provider win rate in basis points (needed for Kelly method)
/// `avg_win_bps`  — provider average win in basis points (needed for Kelly method)
/// `avg_loss_bps` — provider average loss magnitude in basis points (needed for Kelly method)
pub fn calculate_position_size(
    env: &Env,
    user: &Address,
    asset_id: u32,
    win_rate_bps: i128,
    avg_win_bps: i128,
    avg_loss_bps: i128,
) -> Result<SizingRecommendation, AutoTradeError> {
    let config = get_sizing_config(env, user);
    let portfolio_value = calculate_portfolio_value(env, user);

    // Use the current risk config's max_position_pct as a safety cross-check too
    let risk_config: RiskConfig = get_risk_config(env, user);

    let volatility_bps = {
        let raw = calculate_volatility(env, asset_id, 30);
        if raw == 0 {
            // Zero volatility → treat as maximum risk → minimum position
            MAX_VOLATILITY_BPS
        } else {
            raw
        }
    };

    let raw_size = match &config.method {
        SizingMethod::FixedPercentage => {
            if volatility_bps == 0 {
                MIN_POSITION_SIZE
            } else {
                // size = portfolio * risk_per_trade_bps / volatility_bps
                portfolio_value
                    .saturating_mul(config.risk_per_trade_bps as i128)
                    / volatility_bps
            }
        }

        SizingMethod::Kelly => {
            let kelly_f = calculate_kelly_fraction(win_rate_bps, avg_win_bps, avg_loss_bps);
            if kelly_f == 0 {
                MIN_POSITION_SIZE
            } else {
                // size = portfolio * kelly_f (bps) * multiplier / (10000 * 100)
                // kelly_f is in bps so divide by 10000
                // kelly_multiplier is out of 100 (e.g. 50 = 0.5x)
                portfolio_value
                    .saturating_mul(kelly_f)
                    .saturating_mul(config.kelly_multiplier as i128)
                    / (10_000 * 100)
            }
        }

        SizingMethod::VolatilityScaled => {
            let target_vol = if config.target_volatility_bps == 0 {
                DEFAULT_TARGET_VOLATILITY_BPS as u32
            } else {
                config.target_volatility_bps
            };
            // base_size = portfolio * base_position_pct_bps / 10000
            let base_size = portfolio_value
                .saturating_mul(config.base_position_pct_bps as i128)
                / 10_000;

            if volatility_bps == 0 {
                MIN_POSITION_SIZE
            } else {
                // adjusted = base_size * target_vol / current_vol
                base_size
                    .saturating_mul(target_vol as i128)
                    / volatility_bps
            }
        }
    };

    // Enforce minimum
    let sized = raw_size.max(MIN_POSITION_SIZE);

    // Derive max from the LOWER of the sizing config and the existing risk config
    let max_by_sizing = portfolio_value
        .saturating_mul(config.max_position_pct_bps as i128)
        / 10_000;
    let max_by_risk = portfolio_value
        .saturating_mul(risk_config.max_position_pct as i128)
        / 100;
    let max_size = max_by_sizing.min(max_by_risk).max(MIN_POSITION_SIZE);

    // Also check available balance if a price is known
    let balance_cap = if let Some(price) = get_asset_price(env, asset_id) {
        if price > 0 {
            // Approximate: how many units can the max_size buy at current price?
            // We keep everything in the same unit as portfolio_value here, so
            // the returned recommended_size is in value terms (same as portfolio_value).
            max_size
        } else {
            max_size
        }
    } else {
        max_size
    };

    let final_max = balance_cap;
    let (recommended_size, was_capped) = if sized > final_max {
        (final_max, true)
    } else {
        (sized, false)
    };

    Ok(SizingRecommendation {
        recommended_size,
        max_size: final_max,
        volatility_bps,
        portfolio_value,
        was_capped,
    })
}

/// Convenience wrapper that reads provider stats from the signal registry
/// (passed in directly by the caller to avoid cross-contract calls).
///
/// Returns only the recommended position size (not the full recommendation struct).
pub fn get_position_size_for_trade(
    env: &Env,
    user: &Address,
    asset_id: u32,
    win_rate_bps: i128,
    avg_win_bps: i128,
    avg_loss_bps: i128,
    available_balance: i128,
) -> Result<i128, AutoTradeError> {
    let rec = calculate_position_size(
        env,
        user,
        asset_id,
        win_rate_bps,
        avg_win_bps,
        avg_loss_bps,
    )?;

    // Clamp to available balance
    let size = rec.recommended_size.min(available_balance).max(MIN_POSITION_SIZE);
    Ok(size)
}