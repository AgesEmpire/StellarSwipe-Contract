//! Risk Parity Portfolio Rebalancing
//!
//! Equalizes the risk contribution of each asset in a portfolio.
//! High volatility assets receive lower weights, low volatility assets receive higher weights.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, Symbol, Vec};

use crate::errors::AutoTradeError;
use crate::portfolio;
use crate::risk;

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetRisk {
    pub asset_id: u32,
    pub volatility_bps: i128,
    pub current_value_xlm: i128,
    pub current_risk_contribution: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RebalanceTrade {
    pub asset_id: u32,
    pub trade_amount_xlm: i128,
    pub is_buy: bool,
}

/// Calculate risk contributions and recommended trades for risk parity.
pub fn calculate_risk_parity_rebalance(
    env: &Env,
    user: &Address,
) -> Result<(Vec<AssetRisk>, Vec<RebalanceTrade>), AutoTradeError> {
    let portfolio = portfolio::get_portfolio(env, user);
    let config = risk::get_risk_parity_config(env, user);

    if portfolio.assets.is_empty() {
        return Err(AutoTradeError::InvalidAmount);
    }

    let mut asset_risks = Vec::new(env);
    let mut total_risk_contribution = 0i128;

    // 1. Calculate current risk contributions
    for i in 0..portfolio.assets.len() {
        let asset = portfolio.assets.get(i).unwrap();
        let vol = risk::calculate_volatility(env, asset.asset_id, 30);

        // Risk Contribution = Value * Volatility
        let risk_contrib = asset.current_value_xlm * vol;

        asset_risks.push_back(AssetRisk {
            asset_id: asset.asset_id,
            volatility_bps: vol,
            current_value_xlm: asset.current_value_xlm,
            current_risk_contribution: risk_contrib,
        });

        total_risk_contribution += risk_contrib;
    }

    if total_risk_contribution == 0 {
        return Ok((asset_risks, Vec::new(env)));
    }

    // 2. Determine target weights
    // Risk Parity: Inverse volatility weighting
    // Weight_i = (1/Vol_i) / Σ(1/Vol_j)
    let mut inv_vol_sum = 0i128;
    let mut inv_vols = Vec::new(env);

    for i in 0..asset_risks.len() {
        let ar = asset_risks.get(i).unwrap();
        // Use a large constant to maintain precision for inverse: 10^12
        let inv_vol = if ar.volatility_bps > 0 {
            1_000_000_000_000i128 / ar.volatility_bps
        } else {
            1_000_000_000_000i128 / risk::DEFAULT_VOLATILITY_BPS
        };
        inv_vols.push_back(inv_vol);
        inv_vol_sum += inv_vol;
    }

    // 3. Calculate target values and generate trades
    let mut trades = Vec::new(env);
    let threshold_bps = (config.threshold_pct as i128) * 100;
    let total_value = portfolio.total_value_xlm;

    for i in 0..asset_risks.len() {
        let ar = asset_risks.get(i).unwrap();
        let inv_vol = inv_vols.get(i).unwrap();

        // Target Value = Total Portfolio Value * (inv_vol / inv_vol_sum)
        let target_value = (total_value * inv_vol) / inv_vol_sum;

        let diff = target_value - ar.current_value_xlm;
        let abs_diff = if diff < 0 { -diff } else { diff };

        // Check if difference exceeds threshold (e.g. 1% of the asset's current value)
        // or if current value is 0 (new allocation)
        let exceeds_threshold = if ar.current_value_xlm > 0 {
            (abs_diff * 10000 / ar.current_value_xlm) > threshold_bps
        } else {
            abs_diff > 0
        };

        if exceeds_threshold {
            trades.push_back(RebalanceTrade {
                asset_id: ar.asset_id,
                trade_amount_xlm: abs_diff,
                is_buy: diff > 0,
            });
        }
    }

    Ok((asset_risks, trades))
}

/// Execute a risk parity rebalance for a user.
pub fn execute_risk_parity_rebalance(env: &Env, user: &Address) -> Result<(), AutoTradeError> {
    let config = risk::get_risk_parity_config(env, user);

    if !config.enabled {
        return Err(AutoTradeError::Unauthorized);
    }

    // Frequency check
    let now = env.ledger().timestamp();
    let seconds_per_day = 86400;
    if now < config.last_rebalance + (config.rebalance_frequency_days as u64 * seconds_per_day) {
        return Ok(()); // Too soon to rebalance
    }

    let (_, trades) = calculate_risk_parity_rebalance(env, user)?;

    if trades.is_empty() {
        return Ok(());
    }

    // Execute trades (simplified: in real SDEX would need asset IDs to symbols/XDR)
    for i in 0..trades.len() {
        let trade = trades.get(i).unwrap();

        // In a real contract, this would call SDEX to buy/sell
        // For this implementation, we update positions to reflect the rebalance
        let mut positions = risk::get_user_positions(env, user);
        if let Some(mut pos) = positions.get(trade.asset_id) {
            let price = risk::get_asset_price(env, trade.asset_id).unwrap_or(pos.entry_price);
            if price > 0 {
                let amount_change = trade.trade_amount_xlm / price;
                if trade.is_buy {
                    pos.amount += amount_change;
                } else {
                    pos.amount -= amount_change;
                }

                if pos.amount <= 0 {
                    positions.remove(trade.asset_id);
                } else {
                    positions.set(trade.asset_id, pos);
                }
            }
        }
        env.storage()
            .persistent()
            .set(&risk::RiskDataKey::UserPositions(user.clone()), &positions);
    }

    // Update last rebalance time
    let mut new_config = config;
    new_config.last_rebalance = now;
    risk::set_risk_parity_config(env, user, &new_config);

    // Emit event
    env.events().publish(
        (Symbol::new(env, "risk_parity_rebalance"), user.clone()),
        now,
    );

    Ok(())
}
