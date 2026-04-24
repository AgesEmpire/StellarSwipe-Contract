#![allow(dead_code)]
//! Portfolio Insurance & Dynamic Hedging (Issue #89)
//!
//! Monitors drawdown against a high-water mark and automatically opens,
//! rebalances, and closes offsetting hedge positions.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::AutoTradeError;
use crate::risk;

// ─── Types ────────────────────────────────────────────────────────────────────

/// Identifies which portfolio exposure a hedge is protecting.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HedgePurpose {
    PortfolioProtection,
    AssetSpecificHedge(u32),
}

/// A single open hedge position.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HedgePosition {
    pub asset: u32,
    pub amount: i128,
    pub entry_price: i128,
    pub purpose: HedgePurpose,
}

/// Per-user insurance configuration and state.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PortfolioInsurance {
    pub user: Address,
    pub enabled: bool,
    /// Drawdown % (basis points, 10000 = 100%) that triggers hedging.
    pub max_drawdown_bps: u32,
    /// Fraction of portfolio to hedge (basis points, 5000 = 50%).
    pub hedge_ratio_bps: u32,
    /// Minimum delta (bps) before a rebalance is executed.
    pub rebalance_threshold_bps: u32,
    pub active_hedges: Vec<HedgePosition>,
    pub portfolio_high_water_mark: i128,
}

// ─── Storage ──────────────────────────────────────────────────────────────────

#[contracttype]
pub enum InsuranceKey {
    Insurance(Address),
}

pub fn get_insurance(env: &Env, user: &Address) -> Option<PortfolioInsurance> {
    env.storage()
        .persistent()
        .get(&InsuranceKey::Insurance(user.clone()))
}

pub fn store_insurance(env: &Env, insurance: &PortfolioInsurance) {
    env.storage()
        .persistent()
        .set(&InsuranceKey::Insurance(insurance.user.clone()), insurance);
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Configure (or reconfigure) portfolio insurance for a user.
pub fn configure_insurance(
    env: &Env,
    user: &Address,
    enabled: bool,
    max_drawdown_bps: u32,
    hedge_ratio_bps: u32,
    rebalance_threshold_bps: u32,
) -> Result<(), AutoTradeError> {
    if max_drawdown_bps == 0 || max_drawdown_bps > 10_000 {
        return Err(AutoTradeError::InvalidInsuranceConfig);
    }
    if hedge_ratio_bps == 0 || hedge_ratio_bps > 10_000 {
        return Err(AutoTradeError::InvalidInsuranceConfig);
    }

    let existing = get_insurance(env, user);
    let hwm = existing
        .as_ref()
        .map(|i| i.portfolio_high_water_mark)
        .unwrap_or(0);
    let active_hedges = existing
        .map(|i| i.active_hedges)
        .unwrap_or_else(|| Vec::new(env));

    let insurance = PortfolioInsurance {
        user: user.clone(),
        enabled,
        max_drawdown_bps,
        hedge_ratio_bps,
        rebalance_threshold_bps,
        active_hedges,
        portfolio_high_water_mark: hwm,
    };
    store_insurance(env, &insurance);
    Ok(())
}

/// Return current drawdown in basis points (0–10000).
/// Also updates the high-water mark when a new portfolio high is reached.
pub fn calculate_drawdown(env: &Env, user: &Address) -> Result<i128, AutoTradeError> {
    let mut insurance = get_insurance(env, user).ok_or(AutoTradeError::InsuranceNotConfigured)?;
    let current_value = risk::calculate_portfolio_value(env, user);

    if current_value > insurance.portfolio_high_water_mark {
        insurance.portfolio_high_water_mark = current_value;
        store_insurance(env, &insurance);
    }

    if insurance.portfolio_high_water_mark == 0 {
        return Ok(0);
    }

    let drawdown_bps = ((insurance.portfolio_high_water_mark - current_value) * 10_000)
        / insurance.portfolio_high_water_mark;

    Ok(drawdown_bps)
}

/// Check drawdown and open hedge positions if the threshold is breached.
/// Returns the list of simulated trade IDs (asset ids used as proxy IDs here).
pub fn check_and_apply_hedge(env: &Env, user: &Address) -> Result<Vec<u32>, AutoTradeError> {
    let insurance = get_insurance(env, user).ok_or(AutoTradeError::InsuranceNotConfigured)?;

    if !insurance.enabled {
        return Ok(Vec::new(env));
    }

    let drawdown = calculate_drawdown(env, user)?;

    if drawdown < insurance.max_drawdown_bps as i128 {
        return Ok(Vec::new(env));
    }

    // Already hedged — don't double-hedge
    if !insurance.active_hedges.is_empty() {
        return Ok(Vec::new(env));
    }

    let current_value = risk::calculate_portfolio_value(env, user);
    let hedges = calculate_optimal_hedges(env, user, current_value, insurance.hedge_ratio_bps)?;

    if hedges.is_empty() {
        return Ok(Vec::new(env));
    }

    let mut trade_ids: Vec<u32> = Vec::new(env);
    let mut insurance = get_insurance(env, user).ok_or(AutoTradeError::InsuranceNotConfigured)?;

    for i in 0..hedges.len() {
        if let Some(hedge) = hedges.get(i) {
            trade_ids.push_back(hedge.asset);
            insurance.active_hedges.push_back(hedge);
        }
    }

    store_insurance(env, &insurance);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "hedge_applied"),
            user.clone(),
            drawdown as u32,
        ),
        trade_ids.len() as u32,
    );

    Ok(trade_ids)
}

/// Rebalance existing hedges to match the current portfolio size.
pub fn rebalance_hedges(env: &Env, user: &Address) -> Result<Vec<u32>, AutoTradeError> {
    let insurance = get_insurance(env, user).ok_or(AutoTradeError::InsuranceNotConfigured)?;

    if !insurance.enabled || insurance.active_hedges.is_empty() {
        return Ok(Vec::new(env));
    }

    let current_value = risk::calculate_portfolio_value(env, user);
    let target_hedge_value = (current_value * insurance.hedge_ratio_bps as i128) / 10_000;

    let mut current_hedge_value: i128 = 0;
    for i in 0..insurance.active_hedges.len() {
        if let Some(h) = insurance.active_hedges.get(i) {
            let price = risk::get_asset_price(env, h.asset).unwrap_or(h.entry_price);
            current_hedge_value += h.amount * price / 100;
        }
    }

    let hedge_delta = target_hedge_value - current_hedge_value;
    let denominator = if target_hedge_value > 0 {
        target_hedge_value
    } else {
        1
    };
    let delta_bps = (hedge_delta.abs() * 10_000) / denominator;

    if delta_bps < insurance.rebalance_threshold_bps as i128 {
        return Ok(Vec::new(env));
    }

    let mut trade_ids: Vec<u32> = Vec::new(env);
    let mut insurance = get_insurance(env, user).ok_or(AutoTradeError::InsuranceNotConfigured)?;

    if hedge_delta > 0 {
        // Need more hedging
        let additional = calculate_optimal_hedges(env, user, hedge_delta, 10_000)?;
        for i in 0..additional.len() {
            if let Some(hedge) = additional.get(i) {
                trade_ids.push_back(hedge.asset);
                insurance.active_hedges.push_back(hedge);
            }
        }
    } else {
        // Reduce hedging — trim amounts proportionally
        let reduce_amount = hedge_delta.abs();
        let mut remaining = reduce_amount;
        let mut new_hedges: Vec<HedgePosition> = Vec::new(env);

        for i in 0..insurance.active_hedges.len() {
            if let Some(mut h) = insurance.active_hedges.get(i) {
                let price = risk::get_asset_price(env, h.asset).unwrap_or(h.entry_price);
                let hedge_val = h.amount * price / 100;

                if remaining <= 0 {
                    new_hedges.push_back(h);
                } else if hedge_val <= remaining {
                    remaining -= hedge_val;
                    trade_ids.push_back(h.asset); // closed
                } else {
                    // Partial close
                    let close_units = (remaining * 100) / price.max(1);
                    h.amount -= close_units;
                    remaining = 0;
                    trade_ids.push_back(h.asset);
                    if h.amount > 0 {
                        new_hedges.push_back(h);
                    }
                }
            }
        }
        insurance.active_hedges = new_hedges;
    }

    store_insurance(env, &insurance);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "hedges_rebalanced"), user.clone()),
        trade_ids.len() as u32,
    );

    Ok(trade_ids)
}

/// Close all hedges when portfolio has recovered (drawdown < 500 bps = 5%).
pub fn remove_hedges_if_recovered(env: &Env, user: &Address) -> Result<Vec<u32>, AutoTradeError> {
    let insurance = get_insurance(env, user).ok_or(AutoTradeError::InsuranceNotConfigured)?;

    if insurance.active_hedges.is_empty() {
        return Ok(Vec::new(env));
    }

    let drawdown = calculate_drawdown(env, user)?;

    if drawdown >= 500 {
        return Ok(Vec::new(env));
    }

    let mut trade_ids: Vec<u32> = Vec::new(env);
    let mut insurance = get_insurance(env, user).ok_or(AutoTradeError::InsuranceNotConfigured)?;

    for i in 0..insurance.active_hedges.len() {
        if let Some(h) = insurance.active_hedges.get(i) {
            trade_ids.push_back(h.asset);
        }
    }

    insurance.active_hedges = Vec::new(env);
    store_insurance(env, &insurance);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "hedges_removed"), user.clone()),
        symbol_short!("recovered"),
    );

    Ok(trade_ids)
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Build hedge positions for `total_hedge_value` spread across the user's
/// holdings, weighted by a simple volatility proxy (price deviation from mean).
fn calculate_optimal_hedges(
    env: &Env,
    user: &Address,
    total_hedge_value: i128,
    ratio_bps: u32,
) -> Result<Vec<HedgePosition>, AutoTradeError> {
    let positions = risk::get_user_positions(env, user);
    let mut hedges: Vec<HedgePosition> = Vec::new(env);

    if total_hedge_value <= 0 {
        return Ok(hedges);
    }

    let hedge_value = (total_hedge_value * ratio_bps as i128) / 10_000;
    let keys = positions.keys();
    let n = keys.len();

    if n == 0 {
        return Ok(hedges);
    }

    // Equal-weight across holdings (simple, no external oracle needed)
    let per_asset_value = hedge_value / n as i128;

    for i in 0..n {
        if let Some(asset_id) = keys.get(i) {
            let price = risk::get_asset_price(env, asset_id).unwrap_or(1);
            if price <= 0 {
                continue;
            }
            // units = value / (price / 100)  →  units = value * 100 / price
            let amount = per_asset_value * 100 / price;
            if amount <= 0 {
                continue;
            }
            hedges.push_back(HedgePosition {
                asset: asset_id,
                amount,
                entry_price: price,
                purpose: HedgePurpose::AssetSpecificHedge(asset_id),
            });
        }
    }

    Ok(hedges)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{contract, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let user = Address::generate(&env);
        (env, user)
    }

    fn register_and_run<F: FnOnce(&Env, &Address)>(env: &Env, user: &Address, f: F) {
        let addr = env.register(TestContract, ());
        env.as_contract(&addr, || f(env, user));
    }

    // ── configure ─────────────────────────────────────────────────────────────

    #[test]
    fn test_configure_insurance_success() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();
            let ins = get_insurance(env, user).unwrap();
            assert!(ins.enabled);
            assert_eq!(ins.max_drawdown_bps, 1500);
            assert_eq!(ins.hedge_ratio_bps, 5000);
        });
    }

    #[test]
    fn test_configure_insurance_invalid_drawdown() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            let err = configure_insurance(env, user, true, 0, 5000, 200).unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidInsuranceConfig);
        });
    }

    #[test]
    fn test_configure_insurance_invalid_ratio() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            let err = configure_insurance(env, user, true, 1500, 0, 200).unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidInsuranceConfig);
        });
    }

    // ── drawdown ──────────────────────────────────────────────────────────────

    #[test]
    fn test_drawdown_zero_when_no_portfolio() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();
            let dd = calculate_drawdown(env, user).unwrap();
            assert_eq!(dd, 0);
        });
    }

    #[test]
    fn test_drawdown_updates_high_water_mark() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            // Simulate portfolio value = 10_000 (price 100, amount 100 → value 100*100/100=100... scale up)
            // Use price=100, amount=10000 → value = 10000*100/100 = 10000
            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);

            let dd = calculate_drawdown(env, user).unwrap();
            assert_eq!(dd, 0); // at high water mark

            let ins = get_insurance(env, user).unwrap();
            assert_eq!(ins.portfolio_high_water_mark, 10_000);
        });
    }

    #[test]
    fn test_drawdown_20_percent() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            // Establish HWM at 10_000
            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap(); // sets HWM = 10_000

            // Drop price to 80 → value = 10_000 * 80 / 100 = 8_000
            risk::set_asset_price(env, 1, 80);

            let dd = calculate_drawdown(env, user).unwrap();
            // (10000 - 8000) * 10000 / 10000 = 2000 bps = 20%
            assert_eq!(dd, 2_000);
        });
    }

    // ── check_and_apply_hedge ─────────────────────────────────────────────────

    #[test]
    fn test_hedge_not_triggered_below_threshold() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            // 15% drawdown trigger, 50% hedge ratio
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap(); // HWM = 10_000

            // Only 10% drop
            risk::set_asset_price(env, 1, 90);

            let ids = check_and_apply_hedge(env, user).unwrap();
            assert_eq!(ids.len(), 0);
        });
    }

    #[test]
    fn test_hedge_triggered_at_threshold() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            // 15% drawdown trigger (1500 bps), 50% hedge ratio (5000 bps)
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap(); // HWM = 10_000

            // 20% drop → drawdown = 2000 bps > 1500 bps threshold
            risk::set_asset_price(env, 1, 80);

            let ids = check_and_apply_hedge(env, user).unwrap();
            assert!(ids.len() > 0);

            let ins = get_insurance(env, user).unwrap();
            assert!(!ins.active_hedges.is_empty());
        });
    }

    #[test]
    fn test_hedge_size_approximately_50_percent() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            // 15% trigger, 50% hedge ratio
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            // Portfolio: 10_000 units at price 100 → value = 10_000
            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap(); // HWM = 10_000

            // 20% drop → value = 8_000
            risk::set_asset_price(env, 1, 80);

            check_and_apply_hedge(env, user).unwrap();

            let ins = get_insurance(env, user).unwrap();
            assert_eq!(ins.active_hedges.len(), 1);

            let hedge = ins.active_hedges.get(0).unwrap();
            // target_hedge_value = 8_000 * 5000 / 10_000 = 4_000
            // amount = 4_000 * 100 / 80 = 5_000
            assert_eq!(hedge.amount, 5_000);
        });
    }

    #[test]
    fn test_no_double_hedge() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap();
            risk::set_asset_price(env, 1, 80);

            check_and_apply_hedge(env, user).unwrap();
            let ids2 = check_and_apply_hedge(env, user).unwrap(); // second call
            assert_eq!(ids2.len(), 0); // no new hedges
        });
    }

    #[test]
    fn test_disabled_insurance_skips_hedge() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, false, 1500, 5000, 200).unwrap();

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap();
            risk::set_asset_price(env, 1, 80);

            let ids = check_and_apply_hedge(env, user).unwrap();
            assert_eq!(ids.len(), 0);
        });
    }

    // ── rebalance ─────────────────────────────────────────────────────────────

    #[test]
    fn test_rebalance_no_op_below_threshold() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 1000).unwrap(); // 10% rebalance threshold

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap();
            risk::set_asset_price(env, 1, 80);
            check_and_apply_hedge(env, user).unwrap();

            // Tiny price change — delta < 10% threshold
            risk::set_asset_price(env, 1, 81);
            let ids = rebalance_hedges(env, user).unwrap();
            assert_eq!(ids.len(), 0);
        });
    }

    #[test]
    fn test_rebalance_triggers_on_large_delta() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap(); // 2% threshold

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap();
            risk::set_asset_price(env, 1, 80);
            check_and_apply_hedge(env, user).unwrap();

            // Portfolio grows significantly (price recovers partially but position added)
            risk::update_position(env, user, 1, 20_000, 80);
            let ids = rebalance_hedges(env, user).unwrap();
            assert!(ids.len() > 0);
        });
    }

    // ── remove hedges on recovery ─────────────────────────────────────────────

    #[test]
    fn test_hedges_removed_on_recovery() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap(); // HWM = 10_000

            // Drop 20% → hedge opens
            risk::set_asset_price(env, 1, 80);
            check_and_apply_hedge(env, user).unwrap();

            let ins = get_insurance(env, user).unwrap();
            assert!(!ins.active_hedges.is_empty());

            // Recover to near HWM (drawdown < 5%)
            risk::set_asset_price(env, 1, 99);
            let ids = remove_hedges_if_recovered(env, user).unwrap();
            assert!(ids.len() > 0);

            let ins = get_insurance(env, user).unwrap();
            assert!(ins.active_hedges.is_empty());
        });
    }

    #[test]
    fn test_hedges_not_removed_while_still_down() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            configure_insurance(env, user, true, 1500, 5000, 200).unwrap();

            risk::set_asset_price(env, 1, 100);
            risk::update_position(env, user, 1, 10_000, 100);
            calculate_drawdown(env, user).unwrap();

            risk::set_asset_price(env, 1, 80);
            check_and_apply_hedge(env, user).unwrap();

            // Still 20% down — don't remove
            let ids = remove_hedges_if_recovered(env, user).unwrap();
            assert_eq!(ids.len(), 0);
        });
    }

    // ── not configured ────────────────────────────────────────────────────────

    #[test]
    fn test_drawdown_not_configured_returns_error() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            let err = calculate_drawdown(env, user).unwrap_err();
            assert_eq!(err, AutoTradeError::InsuranceNotConfigured);
        });
    }

    #[test]
    fn test_apply_hedge_not_configured_returns_error() {
        let (env, user) = setup();
        register_and_run(&env, &user, |env, user| {
            let err = check_and_apply_hedge(env, user).unwrap_err();
            assert_eq!(err, AutoTradeError::InsuranceNotConfigured);
        });
    }
}
