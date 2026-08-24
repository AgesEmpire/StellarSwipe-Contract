#![allow(dead_code)]
//! Portfolio calculation and P&L tracking.
//!
//! Builds portfolio from positions with current values and unrealized P&L.

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::errors::AutoTradeError;
use crate::risk;

#[contracttype]
#[derive(Clone)]
pub enum PortfolioDataKey {
    PrivacyMode(Address),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetHolding {
    pub asset_id: u32,
    pub amount: i128,
    pub current_value_xlm: i128,
    pub avg_entry_price: i128,
    pub unrealized_pnl: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Portfolio {
    pub assets: Vec<AssetHolding>,
    pub total_value_xlm: i128,
    pub total_pnl: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortfolioComparison {
    pub user_a_pnl: i128,
    pub user_b_pnl: i128,
    pub user_a_roi: i128,
    pub user_b_roi: i128,
    pub user_a_win_rate: u32,
    pub user_b_win_rate: u32,
    pub winner: Address,
}

pub fn set_privacy_mode(env: &Env, user: &Address, enabled: bool) {
    env.storage()
        .persistent()
        .set(&PortfolioDataKey::PrivacyMode(user.clone()), &enabled);
}

pub fn is_privacy_mode_enabled(env: &Env, user: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&PortfolioDataKey::PrivacyMode(user.clone()))
        .unwrap_or(false)
}

/// Get portfolio for user. Uses risk::get_user_positions and risk::get_asset_price.
pub fn get_portfolio(env: &Env, user: &Address) -> Portfolio {
    let positions = risk::get_user_positions(env, user);
    let mut assets = Vec::new(env);
    let mut total_value_xlm = 0i128;
    let mut total_pnl = 0i128;

    let keys = positions.keys();
    for i in 0..keys.len() {
        if let Some(asset_id) = keys.get(i) {
            if let Some(position) = positions.get(asset_id) {
                let current_price =
                    risk::get_asset_price(env, asset_id).unwrap_or(position.entry_price);
                let current_value_xlm = position.amount * current_price;
                let unrealized_pnl = (current_price - position.entry_price) * position.amount;

                total_value_xlm += current_value_xlm;
                total_pnl += unrealized_pnl;

                assets.push_back(AssetHolding {
                    asset_id,
                    amount: position.amount,
                    current_value_xlm,
                    avg_entry_price: position.entry_price,
                    unrealized_pnl,
                });
            }
        }
    }

    Portfolio {
        assets,
        total_value_xlm,
        total_pnl,
    }
}

pub fn compare_portfolios(
    env: &Env,
    user_a: Address,
    user_b: Address,
) -> Result<PortfolioComparison, AutoTradeError> {
    if is_privacy_mode_enabled(env, &user_a) || is_privacy_mode_enabled(env, &user_b) {
        return Err(AutoTradeError::PrivacyModeEnabled);
    }

    let portfolio_a = get_portfolio(env, &user_a);
    let portfolio_b = get_portfolio(env, &user_b);
    let user_a_roi = calculate_roi(&portfolio_a);
    let user_b_roi = calculate_roi(&portfolio_b);
    let user_a_win_rate = calculate_win_rate(&portfolio_a);
    let user_b_win_rate = calculate_win_rate(&portfolio_b);
    let winner = if user_a_roi >= user_b_roi {
        user_a.clone()
    } else {
        user_b.clone()
    };

    Ok(PortfolioComparison {
        user_a_pnl: portfolio_a.total_pnl,
        user_b_pnl: portfolio_b.total_pnl,
        user_a_roi,
        user_b_roi,
        user_a_win_rate,
        user_b_win_rate,
        winner,
    })
}

fn calculate_roi(portfolio: &Portfolio) -> i128 {
    let invested = portfolio.total_value_xlm - portfolio.total_pnl;
    if invested == 0 {
        0
    } else {
        portfolio.total_pnl * 10_000 / invested
    }
}

fn calculate_win_rate(portfolio: &Portfolio) -> u32 {
    let total = portfolio.assets.len();
    if total == 0 {
        return 0;
    }

    let mut wins = 0u32;
    for i in 0..total {
        if let Some(asset) = portfolio.assets.get(i) {
            if asset.unrealized_pnl > 0 {
                wins += 1;
            }
        }
    }

    wins * 10_000 / total
}
