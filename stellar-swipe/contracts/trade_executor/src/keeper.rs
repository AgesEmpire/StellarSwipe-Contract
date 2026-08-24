//! Keeper network interface for trigger monitoring.
//!
//! Keepers are off-chain bots that call `check_and_trigger_stop_loss` /
//! `check_and_trigger_take_profit` on behalf of users.  This module provides:
//!
//! - `get_triggerable_positions()` — scan registered positions and return those
//!   whose oracle price has already crossed the trigger threshold.
//! - `compute_keeper_reward(position_value)` — 0.1% of position value.
//! - `KEEPER_REWARD_BPS` — the reward rate constant (10 bps = 0.1%).

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::ContractError;
use crate::triggers::{get_stop_loss, get_take_profit};

/// Reward paid to the keeper as a fraction of position value (10 bps = 0.1%).
pub const KEEPER_REWARD_BPS: i128 = 10;

/// Identifies which trigger type a position is ready for.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TriggerType {
    StopLoss,
    TakeProfit,
}

/// A position that has crossed its trigger threshold and is ready to be executed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerablePosition {
    pub trade_id: u64,
    pub user: Address,
    pub trigger_type: TriggerType,
    pub trigger_price: i128,
    pub current_price: i128,
}

// ── Storage keys used by keeper module ───────────────────────────────────────

/// Persistent list of all registered (user, trade_id, asset_pair) watch entries.
/// Stored as `Vec<(Address, u64, u32)>`.
pub const WATCH_LIST_KEY: &str = "WatchList";

// ── Public helpers ────────────────────────────────────────────────────────────

/// Register a `(user, trade_id, asset_pair)` tuple so keepers can discover it.
/// Called internally by `set_stop_loss_price` / `set_take_profit_price`.
pub fn register_watch(env: &Env, user: &Address, trade_id: u64, asset_pair: u32) {
    let key = Symbol::new(env, WATCH_LIST_KEY);
    let mut list: Vec<(Address, u64, u32)> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    // Avoid duplicates.
    for i in 0..list.len() {
        if let Some((u, t, _)) = list.get(i) {
            if u == *user && t == trade_id {
                return;
            }
        }
    }
    list.push_back((user.clone(), trade_id, asset_pair));
    env.storage().persistent().set(&key, &list);
}

/// Fetch the current oracle price for `asset_pair`.
/// Returns `None` if the oracle is not configured or the call fails.
fn oracle_price(env: &Env, asset_pair: u32) -> Option<i128> {
    let oracle: Address = env
        .storage()
        .instance()
        .get(&Symbol::new(env, crate::triggers::ORACLE_KEY))?;

    env.try_invoke_contract::<i128, soroban_sdk::Error>(
        &oracle,
        &Symbol::new(env, "get_price"),
        soroban_sdk::vec![env, asset_pair.into()],
    )
    .ok()
    .and_then(|r| r.ok())
}

/// Scan all registered watch entries and return those whose oracle price has
/// already crossed the trigger threshold.
///
/// A stop-loss is triggerable when `current_price <= stop_loss_price`.
/// A take-profit is triggerable when `current_price >= take_profit_price`
/// (and stop-loss is not simultaneously triggered, matching trigger priority).
pub fn get_triggerable_positions(env: &Env) -> Result<Vec<TriggerablePosition>, ContractError> {
    let key = Symbol::new(env, WATCH_LIST_KEY);
    let list: Vec<(Address, u64, u32)> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    let mut result: Vec<TriggerablePosition> = Vec::new(env);

    for i in 0..list.len() {
        let Some((user, trade_id, asset_pair)) = list.get(i) else {
            continue;
        };

        let Some(current_price) = oracle_price(env, asset_pair) else {
            continue;
        };

        // Check stop-loss first (higher priority).
        if let Some(sl_price) = get_stop_loss(env, &user, trade_id) {
            if current_price <= sl_price {
                result.push_back(TriggerablePosition {
                    trade_id,
                    user: user.clone(),
                    trigger_type: TriggerType::StopLoss,
                    trigger_price: sl_price,
                    current_price,
                });
                continue; // stop-loss takes priority; skip take-profit check
            }
        }

        // Check take-profit.
        if let Some(tp_price) = get_take_profit(env, &user, trade_id) {
            if current_price >= tp_price {
                result.push_back(TriggerablePosition {
                    trade_id,
                    user: user.clone(),
                    trigger_type: TriggerType::TakeProfit,
                    trigger_price: tp_price,
                    current_price,
                });
            }
        }
    }

    Ok(result)
}

/// Compute the keeper reward for a given position value.
/// `reward = position_value * KEEPER_REWARD_BPS / 10_000`
/// Returns 0 on overflow or zero value.
pub fn compute_keeper_reward(position_value: i128) -> i128 {
    position_value
        .checked_mul(KEEPER_REWARD_BPS)
        .and_then(|n| n.checked_div(10_000))
        .unwrap_or(0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        triggers::{set_stop_loss, set_take_profit},
        TradeExecutorContract, TradeExecutorContractClient,
    };
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Env};

    // ── Mock oracle ───────────────────────────────────────────────────────────

    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, price: i128) {
            env.storage()
                .instance()
                .set(&symbol_short!("price"), &price);
        }
        pub fn get_price(env: Env, _asset_pair: u32) -> i128 {
            env.storage()
                .instance()
                .get(&symbol_short!("price"))
                .unwrap_or(0)
        }
    }

    // ── Setup helper ──────────────────────────────────────────────────────────

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle_id = env.register(MockOracle, ());
        let exec_id = env.register(TradeExecutorContract, ());

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.initialize(&admin);
        exec.add_oracle(&oracle_id);
        exec.set_oracle(&oracle_id);

        (env, exec_id, oracle_id)
    }

    // ── get_triggerable_positions tests ──────────────────────────────────────

    /// A stop-loss position whose price has crossed is returned.
    #[test]
    fn triggerable_stop_loss_is_returned() {
        let (env, exec_id, oracle_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&exec_id, || {
            set_stop_loss(&env, &user, 1u64, 100);
            register_watch(&env, &user, 1u64, 0u32);
        });

        MockOracleClient::new(&env, &oracle_id).set_price(&80); // below stop-loss

        let positions = env.as_contract(&exec_id, || get_triggerable_positions(&env).unwrap());

        assert_eq!(positions.len(), 1);
        let p = positions.get(0).unwrap();
        assert_eq!(p.trade_id, 1u64);
        assert_eq!(p.trigger_type, TriggerType::StopLoss);
        assert_eq!(p.trigger_price, 100);
        assert_eq!(p.current_price, 80);
    }

    /// A take-profit position whose price has crossed is returned.
    #[test]
    fn triggerable_take_profit_is_returned() {
        let (env, exec_id, oracle_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&exec_id, || {
            set_take_profit(&env, &user, 2u64, 200);
            register_watch(&env, &user, 2u64, 0u32);
        });

        MockOracleClient::new(&env, &oracle_id).set_price(&250); // above take-profit

        let positions = env.as_contract(&exec_id, || get_triggerable_positions(&env).unwrap());

        assert_eq!(positions.len(), 1);
        let p = positions.get(0).unwrap();
        assert_eq!(p.trade_id, 2u64);
        assert_eq!(p.trigger_type, TriggerType::TakeProfit);
        assert_eq!(p.trigger_price, 200);
        assert_eq!(p.current_price, 250);
    }

    /// A position that has not crossed its threshold is NOT returned.
    #[test]
    fn non_triggerable_position_not_returned() {
        let (env, exec_id, oracle_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&exec_id, || {
            set_stop_loss(&env, &user, 3u64, 50);
            register_watch(&env, &user, 3u64, 0u32);
        });

        MockOracleClient::new(&env, &oracle_id).set_price(&120); // above stop-loss, not triggered

        let positions = env.as_contract(&exec_id, || get_triggerable_positions(&env).unwrap());

        assert_eq!(positions.len(), 0);
    }

    // ── Keeper reward calculation tests ──────────────────────────────────────

    /// Reward is 0.1% of position value.
    #[test]
    fn keeper_reward_is_correct() {
        // 1_000_000 * 10 / 10_000 = 1_000
        assert_eq!(compute_keeper_reward(1_000_000), 1_000);
    }

    /// Reward rounds down for small values.
    #[test]
    fn keeper_reward_rounds_down() {
        // 999 * 10 / 10_000 = 0 (integer division)
        assert_eq!(compute_keeper_reward(999), 0);
    }

    /// Zero position value yields zero reward.
    #[test]
    fn keeper_reward_zero_value() {
        assert_eq!(compute_keeper_reward(0), 0);
    }
}
