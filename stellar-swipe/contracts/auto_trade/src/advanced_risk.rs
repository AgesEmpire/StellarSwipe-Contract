#![allow(dead_code)]
use soroban_sdk::{contracttype, symbol_short, Address, Env};

use crate::risk::{self, Position, RiskConfig};

pub const BPS_DENOMINATOR: i128 = 10_000;
pub const MIN_TRAILING_STOP_PCT: u32 = 500;
pub const MAX_TRAILING_STOP_PCT: u32 = 2_500;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopTrigger {
    FixedStopLoss,
    TrailingStop,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoSellResult {
    pub asset_id: u32,
    pub trigger: StopTrigger,
    pub trigger_price: i128,
    pub execution_price: i128,
    pub sold_amount: i128,
    pub remaining_amount: i128,
}

pub fn calculate_trailing_stop(high_price: i128, trailing_stop_pct: u32) -> i128 {
    if high_price <= 0 {
        return 0;
    }

    high_price * (BPS_DENOMINATOR - trailing_stop_pct as i128) / BPS_DENOMINATOR
}

pub fn get_position(env: &Env, user: &Address, asset_id: u32) -> Option<Position> {
    risk::get_user_positions(env, user).get(asset_id)
}

pub fn update_position_high(
    env: &Env,
    user: &Address,
    asset_id: u32,
    current_price: i128,
) -> Option<Position> {
    let mut positions = risk::get_user_positions(env, user);
    let mut position = positions.get(asset_id)?;

    if current_price > position.high_price {
        position.high_price = current_price;
        positions.set(asset_id, position.clone());
        env.storage()
            .persistent()
            .set(&risk::RiskDataKey::UserPositions(user.clone()), &positions);
    }

    Some(position)
}

pub fn get_trailing_stop_price(
    env: &Env,
    user: &Address,
    asset_id: u32,
    config: &RiskConfig,
) -> Option<i128> {
    if !config.trailing_stop_enabled {
        return None;
    }

    let position = get_position(env, user, asset_id)?;
    let high = if position.high_price > 0 {
        position.high_price
    } else {
        position.entry_price
    };

    Some(calculate_trailing_stop(high, config.trailing_stop_pct))
}

pub fn get_fixed_stop_price(position: &Position, config: &RiskConfig) -> i128 {
    position.entry_price * (100 - config.stop_loss_pct as i128) / 100
}

pub fn resolve_stop_trigger(
    position: &Position,
    current_price: i128,
    config: &RiskConfig,
) -> Option<(StopTrigger, i128)> {
    if config.trailing_stop_enabled {
        let high = if position.high_price > 0 {
            position.high_price
        } else {
            position.entry_price
        };
        let trailing_stop = calculate_trailing_stop(high, config.trailing_stop_pct);
        if current_price <= trailing_stop {
            return Some((StopTrigger::TrailingStop, trailing_stop));
        }
    }

    let fixed_stop = get_fixed_stop_price(position, config);
    if current_price <= fixed_stop {
        return Some((StopTrigger::FixedStopLoss, fixed_stop));
    }

    None
}

pub fn process_price_update(
    env: &Env,
    user: &Address,
    asset_id: u32,
    current_price: i128,
) -> Option<AutoSellResult> {
    risk::set_asset_price(env, asset_id, current_price);
    risk::record_price(env, asset_id, current_price);

    let config = risk::get_risk_config(env, user);
    let _ = update_position_high(env, user, asset_id, current_price);
    let position = get_position(env, user, asset_id)?;
    let (trigger, trigger_price) = resolve_stop_trigger(&position, current_price, &config)?;

    let liquidity_key = (symbol_short!("asset_liq"), asset_id);
    let available_liquidity: i128 = env
        .storage()
        .temporary()
        .get(&liquidity_key)
        .unwrap_or(position.amount);
    let sold_amount = core::cmp::min(position.amount, available_liquidity.max(0));
    let remaining_amount = position.amount - sold_amount;

    if remaining_amount != position.amount {
        risk::update_position(env, user, asset_id, remaining_amount, current_price);
    }

    Some(AutoSellResult {
        asset_id,
        trigger,
        trigger_price,
        execution_price: current_price,
        sold_amount,
        remaining_amount,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as TestAddress, Ledger};
    use soroban_sdk::{contract, Address, Env};

    #[contract]
    struct TestContract;

    fn setup_env() -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        env
    }

    fn test_user(env: &Env) -> Address {
        Address::generate(env)
    }

    #[test]
    fn test_calculate_trailing_stop() {
        assert_eq!(calculate_trailing_stop(200, 1000), 180);
        assert_eq!(calculate_trailing_stop(150, 500), 142);
    }

    #[test]
    fn test_process_price_update_triggers_trailing_stop() {
        let env = setup_env();
        let user = test_user(&env);
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            risk::set_risk_config(
                &env,
                &user,
                &RiskConfig {
                    max_position_pct: 20,
                    daily_trade_limit: 10,
                    stop_loss_pct: 15,
                    trailing_stop_enabled: true,
                    trailing_stop_pct: 1000,
                },
            );
            risk::update_position(&env, &user, 1, 1_000, 100);
            update_position_high(&env, &user, 1, 200);

            let result = process_price_update(&env, &user, 1, 180).unwrap();
            assert_eq!(result.trigger, StopTrigger::TrailingStop);
            assert_eq!(result.sold_amount, 1_000);
            assert_eq!(result.remaining_amount, 0);
        });
    }
}
