#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::AutoTradeError;

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakeProfitTier {
    pub price: i128,
    pub position_pct: u32, // basis points (10000 = 100%)
    pub executed: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopLossTier {
    pub trigger_profit_pct: u32, // activate after this % profit (0-based)
    pub trail_pct: u32,          // trail distance in %
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StrategyStatus {
    Active,
    StopHit,
    Complete,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitStrategy {
    pub user: Address,
    pub signal_id: u64,
    pub entry_price: i128,
    pub current_position_size: i128,
    pub take_profit_tiers: Vec<TakeProfitTier>,
    pub stop_loss_tiers: Vec<StopLossTier>,
    pub highest_price: i128, // tracks peak for trailing stop
    pub status: StrategyStatus,
}

#[contracttype]
pub enum ExitStrategyKey {
    Strategy(u64),
    NextId,
    UserStrategies(Address),
}

// ── Storage ───────────────────────────────────────────────────────────────────

fn next_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .persistent()
        .get(&ExitStrategyKey::NextId)
        .unwrap_or(0u64);
    env.storage()
        .persistent()
        .set(&ExitStrategyKey::NextId, &(id + 1));
    id
}

fn save(env: &Env, id: u64, s: &ExitStrategy) {
    env.storage()
        .persistent()
        .set(&ExitStrategyKey::Strategy(id), s);
}

fn load(env: &Env, id: u64) -> Result<ExitStrategy, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&ExitStrategyKey::Strategy(id))
        .ok_or(AutoTradeError::ExitStrategyNotFound)
}

fn add_user_strategy(env: &Env, user: &Address, id: u64) {
    let mut ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&ExitStrategyKey::UserStrategies(user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    ids.push_back(id);
    env.storage()
        .persistent()
        .set(&ExitStrategyKey::UserStrategies(user.clone()), &ids);
}

// ── SDEX sell stub (mirrors sdex.rs pattern) ──────────────────────────────────

fn execute_sell(env: &Env, _user: &Address, signal_id: u64, amount: i128, price: i128) -> u64 {
    // Deterministic synthetic trade id
    let seq = env.ledger().sequence();
    (signal_id)
        .wrapping_mul(1_000_000_007)
        .wrapping_add(amount as u64)
        .wrapping_add(price as u64)
        .wrapping_add(seq as u64)
}

// ── Trailing stop calculation ─────────────────────────────────────────────────

fn calculate_trailing_stop(highest_price: i128, trail_pct: u32) -> i128 {
    highest_price * (100 - trail_pct as i128) / 100
}

// ── Core execution ────────────────────────────────────────────────────────────

pub fn check_and_execute_exits(
    env: &Env,
    strategy_id: u64,
    current_price: i128,
) -> Result<Vec<u64>, AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;

    if strategy.status != StrategyStatus::Active {
        return Ok(Vec::new(env));
    }

    // Update highest price for trailing stop
    if current_price > strategy.highest_price {
        strategy.highest_price = current_price;
    }

    let mut executed_trades: Vec<u64> = Vec::new(env);

    // ── Take-profit tiers ─────────────────────────────────────────────────────
    let tp_len = strategy.take_profit_tiers.len();
    for i in 0..tp_len {
        let tp = strategy.take_profit_tiers.get(i).unwrap();
        if !tp.executed && current_price >= tp.price && strategy.current_position_size > 0 {
            let close_amount =
                (strategy.current_position_size * tp.position_pct as i128) / 10_000;
            let close_amount = close_amount.max(1);

            let trade_id = execute_sell(
                env,
                &strategy.user,
                strategy.signal_id,
                close_amount,
                current_price,
            );

            strategy.current_position_size -= close_amount;

            let mut updated_tp = tp.clone();
            updated_tp.executed = true;
            strategy.take_profit_tiers.set(i, updated_tp);

            executed_trades.push_back(trade_id);

            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(env, "tp_hit"),
                    strategy_id,
                    tp.price,
                ),
                (close_amount, strategy.current_position_size),
            );
        }
    }

    // ── Trailing stop tiers ───────────────────────────────────────────────────
    if strategy.current_position_size > 0 {
        let current_profit_pct = if strategy.entry_price > 0 {
            ((current_price - strategy.entry_price) * 100) / strategy.entry_price
        } else {
            0
        };

        // Activate tiers whose profit threshold has been crossed
        let sl_len = strategy.stop_loss_tiers.len();
        for i in 0..sl_len {
            let mut tier = strategy.stop_loss_tiers.get(i).unwrap();
            if current_profit_pct >= tier.trigger_profit_pct as i128 {
                tier.active = true;
                strategy.stop_loss_tiers.set(i, tier);
            }
        }

        // Find tightest active trail_pct
        let mut tightest_trail: Option<u32> = None;
        for i in 0..sl_len {
            let tier = strategy.stop_loss_tiers.get(i).unwrap();
            if tier.active {
                tightest_trail = Some(match tightest_trail {
                    None => tier.trail_pct,
                    Some(prev) => prev.min(tier.trail_pct),
                });
            }
        }

        if let Some(trail_pct) = tightest_trail {
            let stop_price = calculate_trailing_stop(strategy.highest_price, trail_pct);
            if current_price <= stop_price && strategy.current_position_size > 0 {
                let trade_id = execute_sell(
                    env,
                    &strategy.user,
                    strategy.signal_id,
                    strategy.current_position_size,
                    current_price,
                );

                #[allow(deprecated)]
                env.events().publish(
                    (Symbol::new(env, "trail_stop_hit"), strategy_id),
                    (current_price, strategy.current_position_size),
                );

                strategy.current_position_size = 0;
                strategy.status = StrategyStatus::StopHit;
                executed_trades.push_back(trade_id);
            }
        }
    }

    // ── Mark complete ─────────────────────────────────────────────────────────
    if strategy.current_position_size == 0 && strategy.status == StrategyStatus::Active {
        strategy.status = StrategyStatus::Complete;
    }

    save(env, strategy_id, &strategy);
    Ok(executed_trades)
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn create_exit_strategy(
    env: &Env,
    user: Address,
    signal_id: u64,
    entry_price: i128,
    position_size: i128,
    take_profit_tiers: Vec<TakeProfitTier>,
    stop_loss_tiers: Vec<StopLossTier>,
) -> Result<u64, AutoTradeError> {
    if entry_price <= 0 || position_size <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    if take_profit_tiers.is_empty() {
        return Err(AutoTradeError::InvalidExitConfig);
    }

    let id = next_id(env);
    let strategy = ExitStrategy {
        user: user.clone(),
        signal_id,
        entry_price,
        current_position_size: position_size,
        take_profit_tiers,
        stop_loss_tiers,
        highest_price: entry_price,
        status: StrategyStatus::Active,
    };

    save(env, id, &strategy);
    add_user_strategy(env, &user, id);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "exit_strategy_created"), user, signal_id),
        (id, entry_price, position_size),
    );

    Ok(id)
}

pub fn get_exit_strategy(env: &Env, strategy_id: u64) -> Result<ExitStrategy, AutoTradeError> {
    load(env, strategy_id)
}

pub fn get_user_exit_strategies(env: &Env, user: &Address) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&ExitStrategyKey::UserStrategies(user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// Adjust remaining position size (e.g. after manual partial close).
pub fn adjust_position_size(
    env: &Env,
    user: &Address,
    strategy_id: u64,
    new_size: i128,
) -> Result<(), AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;
    if strategy.user != *user {
        return Err(AutoTradeError::Unauthorized);
    }
    if new_size < 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    strategy.current_position_size = new_size;
    if new_size == 0 {
        strategy.status = StrategyStatus::Complete;
    }
    save(env, strategy_id, &strategy);
    Ok(())
}

// ── Preset strategies ─────────────────────────────────────────────────────────

/// Conservative: TP at +20%/+50%/+100%, trail 10% from start.
pub fn preset_conservative(
    env: &Env,
    user: Address,
    signal_id: u64,
    entry_price: i128,
    position_size: i128,
) -> Result<u64, AutoTradeError> {
    let mut tps = Vec::new(env);
    tps.push_back(TakeProfitTier {
        price: entry_price * 120 / 100,
        position_pct: 3_333,
        executed: false,
    });
    tps.push_back(TakeProfitTier {
        price: entry_price * 150 / 100,
        position_pct: 5_000,
        executed: false,
    });
    tps.push_back(TakeProfitTier {
        price: entry_price * 200 / 100,
        position_pct: 10_000,
        executed: false,
    });

    let mut sls = Vec::new(env);
    sls.push_back(StopLossTier {
        trigger_profit_pct: 0,
        trail_pct: 10,
        active: true,
    });

    create_exit_strategy(env, user, signal_id, entry_price, position_size, tps, sls)
}

/// Balanced: TP at +30%/+80%, tiered trails 10%→7%.
pub fn preset_balanced(
    env: &Env,
    user: Address,
    signal_id: u64,
    entry_price: i128,
    position_size: i128,
) -> Result<u64, AutoTradeError> {
    let mut tps = Vec::new(env);
    tps.push_back(TakeProfitTier {
        price: entry_price * 130 / 100,
        position_pct: 5_000,
        executed: false,
    });
    tps.push_back(TakeProfitTier {
        price: entry_price * 180 / 100,
        position_pct: 10_000,
        executed: false,
    });

    let mut sls = Vec::new(env);
    sls.push_back(StopLossTier {
        trigger_profit_pct: 0,
        trail_pct: 10,
        active: true,
    });
    sls.push_back(StopLossTier {
        trigger_profit_pct: 20,
        trail_pct: 7,
        active: false,
    });

    create_exit_strategy(env, user, signal_id, entry_price, position_size, tps, sls)
}

/// Aggressive: TP at +15%/+30%/+60%/+150%, tight trail 5% after 50% profit.
pub fn preset_aggressive(
    env: &Env,
    user: Address,
    signal_id: u64,
    entry_price: i128,
    position_size: i128,
) -> Result<u64, AutoTradeError> {
    let mut tps = Vec::new(env);
    tps.push_back(TakeProfitTier {
        price: entry_price * 115 / 100,
        position_pct: 2_500,
        executed: false,
    });
    tps.push_back(TakeProfitTier {
        price: entry_price * 130 / 100,
        position_pct: 3_333,
        executed: false,
    });
    tps.push_back(TakeProfitTier {
        price: entry_price * 160 / 100,
        position_pct: 5_000,
        executed: false,
    });
    tps.push_back(TakeProfitTier {
        price: entry_price * 250 / 100,
        position_pct: 10_000,
        executed: false,
    });

    let mut sls = Vec::new(env);
    sls.push_back(StopLossTier {
        trigger_profit_pct: 0,
        trail_pct: 10,
        active: true,
    });
    sls.push_back(StopLossTier {
        trigger_profit_pct: 20,
        trail_pct: 7,
        active: false,
    });
    sls.push_back(StopLossTier {
        trigger_profit_pct: 50,
        trail_pct: 5,
        active: false,
    });

    create_exit_strategy(env, user, signal_id, entry_price, position_size, tps, sls)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract,
        testutils::{Address as _, Ledger as _},
        Env,
    };

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let cid = env.register(TestContract, ());
        (env, cid)
    }

    // ── Conservative preset: 3 TP tiers ──────────────────────────────────────

    #[test]
    fn test_conservative_tp1_partial_close() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            // entry = 1000, position = 10_000
            let id = preset_conservative(&env, user, 1, 1_000, 10_000).unwrap();

            // TP1 at +20% = 1200
            let trades = check_and_execute_exits(&env, id, 1_200).unwrap();
            assert_eq!(trades.len(), 1);

            let s = get_exit_strategy(&env, id).unwrap();
            // 33.33% of 10_000 = 3_333 closed
            assert_eq!(s.current_position_size, 10_000 - 3_333);
            assert_eq!(s.status, StrategyStatus::Active);
        });
    }

    #[test]
    fn test_conservative_tp2_closes_half_remaining() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            let id = preset_conservative(&env, user, 1, 1_000, 10_000).unwrap();

            // Hit TP1 + TP2 in one price update (price gaps to 1500)
            let trades = check_and_execute_exits(&env, id, 1_500).unwrap();
            assert_eq!(trades.len(), 2);

            let s = get_exit_strategy(&env, id).unwrap();
            // TP1: close 3333 → remaining 6667
            // TP2: close 50% of 6667 = 3333 → remaining 3334
            assert_eq!(s.current_position_size, 3_334);
        });
    }

    #[test]
    fn test_conservative_all_tps_complete() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            let id = preset_conservative(&env, user, 1, 1_000, 10_000).unwrap();

            // Price hits all 3 TPs at once
            let trades = check_and_execute_exits(&env, id, 2_000).unwrap();
            assert_eq!(trades.len(), 3);

            let s = get_exit_strategy(&env, id).unwrap();
            assert_eq!(s.current_position_size, 0);
            assert_eq!(s.status, StrategyStatus::Complete);
        });
    }

    // ── Trailing stop ─────────────────────────────────────────────────────────

    #[test]
    fn test_trailing_stop_triggers_before_tp() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            // entry = 1000, trail 10% from start
            let id = preset_conservative(&env, user, 1, 1_000, 10_000).unwrap();

            // Price rises to 1100 (no TP hit), then drops 10% → stop at 990
            check_and_execute_exits(&env, id, 1_100).unwrap();
            let trades = check_and_execute_exits(&env, id, 990).unwrap();
            assert_eq!(trades.len(), 1);

            let s = get_exit_strategy(&env, id).unwrap();
            assert_eq!(s.current_position_size, 0);
            assert_eq!(s.status, StrategyStatus::StopHit);
        });
    }

    #[test]
    fn test_trailing_stop_tightens_after_profit_threshold() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            // Balanced: trail 10% initially, tightens to 7% after 20% profit
            let id = preset_balanced(&env, user, 1, 1_000, 10_000).unwrap();

            // Price rises to 1200 (+20%) → tier 2 activates (trail 7%)
            check_and_execute_exits(&env, id, 1_200).unwrap();

            // Drop to 1116 = 1200 * 93% → within 7% trail, no stop
            let trades = check_and_execute_exits(&env, id, 1_116).unwrap();
            assert_eq!(trades.len(), 0);

            // Drop to 1115 = just below 7% trail of 1200 (1200*0.93=1116)
            let trades = check_and_execute_exits(&env, id, 1_115).unwrap();
            assert_eq!(trades.len(), 1);

            let s = get_exit_strategy(&env, id).unwrap();
            assert_eq!(s.status, StrategyStatus::StopHit);
        });
    }

    // ── Aggressive preset ─────────────────────────────────────────────────────

    #[test]
    fn test_aggressive_four_tps() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            let id = preset_aggressive(&env, user, 1, 1_000, 10_000).unwrap();

            // Hit all 4 TPs
            let trades = check_and_execute_exits(&env, id, 2_500).unwrap();
            assert_eq!(trades.len(), 4);

            let s = get_exit_strategy(&env, id).unwrap();
            assert_eq!(s.current_position_size, 0);
            assert_eq!(s.status, StrategyStatus::Complete);
        });
    }

    // ── Manual position adjustment ────────────────────────────────────────────

    #[test]
    fn test_adjust_position_size() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            let id = preset_conservative(&env, user.clone(), 1, 1_000, 10_000).unwrap();

            adjust_position_size(&env, &user, id, 5_000).unwrap();
            let s = get_exit_strategy(&env, id).unwrap();
            assert_eq!(s.current_position_size, 5_000);
        });
    }

    #[test]
    fn test_adjust_position_to_zero_marks_complete() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            let id = preset_conservative(&env, user.clone(), 1, 1_000, 10_000).unwrap();

            adjust_position_size(&env, &user, id, 0).unwrap();
            let s = get_exit_strategy(&env, id).unwrap();
            assert_eq!(s.status, StrategyStatus::Complete);
        });
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn test_no_execution_on_inactive_strategy() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            let id = preset_conservative(&env, user.clone(), 1, 1_000, 10_000).unwrap();

            // Close all via manual adjust
            adjust_position_size(&env, &user, id, 0).unwrap();

            // Further price checks should return empty
            let trades = check_and_execute_exits(&env, id, 5_000).unwrap();
            assert_eq!(trades.len(), 0);
        });
    }

    #[test]
    fn test_get_user_strategies() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            preset_conservative(&env, user.clone(), 1, 1_000, 10_000).unwrap();
            preset_balanced(&env, user.clone(), 2, 2_000, 5_000).unwrap();

            let ids = get_user_exit_strategies(&env, &user);
            assert_eq!(ids.len(), 2);
        });
    }

    #[test]
    fn test_invalid_entry_price_rejected() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let user = Address::generate(&env);
            let err = preset_conservative(&env, user, 1, 0, 10_000).unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidAmount);
        });
    }
}
