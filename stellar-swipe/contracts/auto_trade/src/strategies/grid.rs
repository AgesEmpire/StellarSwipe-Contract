#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, Symbol, Vec};

use crate::errors::AutoTradeError;

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridConfig {
    pub upper_price: i128,
    pub lower_price: i128,
    pub num_grids: u32,
    pub grid_spacing: i128,
    pub order_size_per_grid: i128,
    pub rebalance_on_fill: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderStatus {
    Open,
    Filled,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridOrder {
    pub level: u32,
    pub price: i128,
    pub order_type: OrderSide,
    pub amount: i128,
    pub order_id: u64,
    pub status: OrderStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilledGridOrder {
    pub level: u32,
    pub price: i128,
    pub order_type: OrderSide,
    pub amount: i128,
    pub filled_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridStatus {
    Initializing,
    Active,
    Stopped,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridStrategy {
    pub user: Address,
    pub asset_pair: u32, // asset id, mirrors existing codebase convention
    pub grid_config: GridConfig,
    pub active_orders: Map<u64, GridOrder>, // grid level → order
    pub filled_orders: Vec<FilledGridOrder>,
    pub total_profit: i128,
    pub status: GridStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridPerformance {
    pub total_profit: i128,
    pub roi_pct: u32, // basis points (×100)
    pub total_fills: u32,
    pub fill_rate: u32, // basis points (×100)
    pub avg_profit_per_fill: i128,
    pub active_orders: u32,
}

#[contracttype]
pub enum GridDataKey {
    Strategy(u64),
    NextId,
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn next_strategy_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .persistent()
        .get(&GridDataKey::NextId)
        .unwrap_or(0u64);
    env.storage()
        .persistent()
        .set(&GridDataKey::NextId, &(id + 1));
    id
}

fn save(env: &Env, id: u64, strategy: &GridStrategy) {
    env.storage()
        .persistent()
        .set(&GridDataKey::Strategy(id), strategy);
}

fn load(env: &Env, id: u64) -> Result<GridStrategy, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&GridDataKey::Strategy(id))
        .ok_or(AutoTradeError::SignalNotFound) // reuse closest existing error
}

// ── Price oracle (mirrors sdex.rs pattern) ───────────────────────────────────

fn get_current_price(env: &Env, asset_pair: u32) -> i128 {
    env.storage()
        .temporary()
        .get(&(symbol_short!("price"), asset_pair))
        .unwrap_or(0)
}

/// Test helper: seed a mock price for an asset pair.
pub fn set_mock_price(env: &Env, asset_pair: u32, price: i128) {
    env.storage()
        .temporary()
        .set(&(symbol_short!("price"), asset_pair), &price);
}

// ── Order placement stubs (mirrors sdex.rs pattern) ──────────────────────────

fn place_limit_order(
    env: &Env,
    _user: &Address,
    asset_pair: u32,
    _side: &OrderSide,
    _amount: i128,
    price: i128,
) -> u64 {
    // Deterministic synthetic order id: hash of (asset_pair, price, ledger seq)
    let seq = env.ledger().sequence();
    let raw = (asset_pair as u64)
        .wrapping_mul(1_000_000_007)
        .wrapping_add(price as u64)
        .wrapping_add(seq as u64);
    raw
}

fn cancel_order(_env: &Env, _order_id: u64) {
    // On-chain cancellation would call SDEX; stubbed here.
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Initialise a new grid strategy and return its id.
pub fn initialize_grid_strategy(
    env: &Env,
    user: Address,
    asset_pair: u32,
    upper_price: i128,
    lower_price: i128,
    num_grids: u32,
    total_capital: i128,
) -> Result<u64, AutoTradeError> {
    if upper_price <= lower_price {
        return Err(AutoTradeError::InvalidAmount);
    }
    if num_grids < 3 || num_grids > 50 {
        return Err(AutoTradeError::InvalidAmount);
    }

    let grid_spacing = (upper_price - lower_price) / (num_grids - 1) as i128;
    let order_size_per_grid = total_capital / num_grids as i128;

    let strategy = GridStrategy {
        user: user.clone(),
        asset_pair,
        grid_config: GridConfig {
            upper_price,
            lower_price,
            num_grids,
            grid_spacing,
            order_size_per_grid,
            rebalance_on_fill: true,
        },
        active_orders: Map::new(env),
        filled_orders: Vec::new(env),
        total_profit: 0,
        status: GridStatus::Initializing,
    };

    let id = next_strategy_id(env);
    save(env, id, &strategy);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "grid_init"), user, asset_pair),
        (id, num_grids, lower_price, upper_price),
    );

    Ok(id)
}

/// Place limit orders at every grid level relative to the current price.
pub fn place_grid_orders(env: &Env, strategy_id: u64) -> Result<(), AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;
    let current_price = get_current_price(env, strategy.asset_pair);

    for level in 0..strategy.grid_config.num_grids {
        let grid_price =
            strategy.grid_config.lower_price + (level as i128 * strategy.grid_config.grid_spacing);

        let order_type = if grid_price < current_price {
            OrderSide::Buy
        } else if grid_price > current_price {
            OrderSide::Sell
        } else {
            continue; // skip exact current price
        };

        let order_id = place_limit_order(
            env,
            &strategy.user,
            strategy.asset_pair,
            &order_type,
            strategy.grid_config.order_size_per_grid,
            grid_price,
        );

        strategy.active_orders.set(
            level as u64,
            GridOrder {
                level,
                price: grid_price,
                order_type,
                amount: strategy.grid_config.order_size_per_grid,
                order_id,
                status: OrderStatus::Open,
            },
        );
    }

    strategy.status = GridStatus::Active;
    save(env, strategy_id, &strategy);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "grid_placed"), strategy_id),
        strategy.active_orders.len(),
    );

    Ok(())
}

/// Called when an order at a grid level is filled.
pub fn on_grid_order_filled(
    env: &Env,
    strategy_id: u64,
    order_id: u64,
    fill_price: i128,
    fill_amount: i128,
) -> Result<(), AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;

    // Find the filled level
    let mut filled_level: Option<u64> = None;
    let keys = strategy.active_orders.keys();
    for i in 0..keys.len() {
        let lvl = keys.get(i).unwrap();
        if let Some(order) = strategy.active_orders.get(lvl) {
            if order.order_id == order_id {
                filled_level = Some(lvl);
                break;
            }
        }
    }

    let level = filled_level.ok_or(AutoTradeError::SignalNotFound)?;
    let filled_order = strategy
        .active_orders
        .get(level)
        .ok_or(AutoTradeError::SignalNotFound)?;
feat/smart-order-routing-84

    let filled_for_profit = filled_order.clone();
 main
    strategy.active_orders.remove(level);

    strategy.filled_orders.push_back(FilledGridOrder {
        level: filled_order.level,
        price: fill_price,
        order_type: filled_order.order_type.clone(),
        amount: fill_amount,
        filled_at: env.ledger().timestamp(),
    });

 feat/smart-order-routing-84
    if let Some(profit) = calculate_grid_profit(&strategy, &filled_order, fill_price, fill_amount) {

    if let Some(profit) =
        calculate_grid_profit(&strategy, &filled_for_profit, fill_price, fill_amount)
    {
 main
        strategy.total_profit += profit;

        #[allow(deprecated)]
        env.events().publish(
 feat/smart-order-routing-84
            (
                Symbol::new(env, "grid_profit"),
                strategy_id,
                filled_order.level,
            ),

            (Symbol::new(env, "grid_profit"), strategy_id, filled_for_profit.level),
main
            profit,
        );
    }

    save(env, strategy_id, &strategy);

    if strategy.grid_config.rebalance_on_fill {
        rebalance_grid_level(env, strategy_id, level)?;
    }

    Ok(())
}

/// Place a replacement order at the filled level based on current price.
pub fn rebalance_grid_level(
    env: &Env,
    strategy_id: u64,
    filled_level: u64,
) -> Result<(), AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;

    let grid_price = strategy.grid_config.lower_price
        + (filled_level as i128 * strategy.grid_config.grid_spacing);

    let current_price = get_current_price(env, strategy.asset_pair);

    let new_order_type = if grid_price < current_price {
        OrderSide::Buy
    } else {
        OrderSide::Sell
    };

    let order_id = place_limit_order(
        env,
        &strategy.user,
        strategy.asset_pair,
        &new_order_type,
        strategy.grid_config.order_size_per_grid,
        grid_price,
    );

    strategy.active_orders.set(
        filled_level,
        GridOrder {
            level: filled_level as u32,
            price: grid_price,
            order_type: new_order_type,
            amount: strategy.grid_config.order_size_per_grid,
            order_id,
            status: OrderStatus::Open,
        },
    );

    save(env, strategy_id, &strategy);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "grid_rebalance"), strategy_id),
        filled_level as u32,
    );

    Ok(())
}

/// Shift the entire grid if price moves outside the configured range.
pub fn adjust_grid_to_price_movement(env: &Env, strategy_id: u64) -> Result<(), AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;
    let current_price = get_current_price(env, strategy.asset_pair);

    if current_price > strategy.grid_config.upper_price
        || current_price < strategy.grid_config.lower_price
    {
        // Cancel all active orders
        let keys = strategy.active_orders.keys();
        for i in 0..keys.len() {
            let lvl = keys.get(i).unwrap();
            if let Some(order) = strategy.active_orders.get(lvl) {
                cancel_order(env, order.order_id);
            }
        }
        strategy.active_orders = Map::new(env);

        // Re-centre grid around current price
        let price_range = strategy.grid_config.upper_price - strategy.grid_config.lower_price;
        strategy.grid_config.upper_price = current_price + (price_range / 2);
        strategy.grid_config.lower_price = current_price - (price_range / 2);
        strategy.grid_config.grid_spacing =
            price_range / (strategy.grid_config.num_grids - 1) as i128;

        save(env, strategy_id, &strategy);

        place_grid_orders(env, strategy_id)?;

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "grid_adjusted"), strategy_id),
            (
                strategy.grid_config.lower_price,
                strategy.grid_config.upper_price,
            ),
        );
    }

    Ok(())
}

/// Return performance metrics for a grid strategy.
pub fn calculate_grid_performance(
    env: &Env,
    strategy_id: u64,
) -> Result<GridPerformance, AutoTradeError> {
    let strategy = load(env, strategy_id)?;

    let total_fills = strategy.filled_orders.len();
    let total_capital =
        strategy.grid_config.order_size_per_grid * strategy.grid_config.num_grids as i128;

    let roi_pct = if total_capital > 0 {
        ((strategy.total_profit * 10_000) / total_capital) as u32
    } else {
        0
    };

    let expected_fills = strategy.grid_config.num_grids * 2;
    let fill_rate = if expected_fills > 0 {
        ((total_fills * 10_000) / expected_fills) as u32
    } else {
        0
    };

    let avg_profit_per_fill = if total_fills > 0 {
        strategy.total_profit / total_fills as i128
    } else {
        0
    };

    Ok(GridPerformance {
        total_profit: strategy.total_profit,
        roi_pct,
        total_fills,
        fill_rate,
        avg_profit_per_fill,
        active_orders: strategy.active_orders.len(),
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn calculate_grid_profit(
    strategy: &GridStrategy,
    filled_order: &GridOrder,
    fill_price: i128,
    fill_amount: i128,
) -> Option<i128> {
    let opposite_type = match filled_order.order_type {
        OrderSide::Buy => OrderSide::Sell,
        OrderSide::Sell => OrderSide::Buy,
    };

    let len = strategy.filled_orders.len();
    // Iterate in reverse to find the most recent matching opposite fill
    for i in (0..len).rev() {
        let prev = strategy.filled_orders.get(i).unwrap();
        if prev.order_type == opposite_type
            && (prev.level as i32 - filled_order.level as i32).abs() == 1
        {
            let profit = match filled_order.order_type {
                OrderSide::Buy => {
                    if fill_price > 0 {
                        (prev.price - fill_price) * fill_amount / fill_price
                    } else {
                        0
                    }
                }
                OrderSide::Sell => {
                    if prev.price > 0 {
                        (fill_price - prev.price) * fill_amount / prev.price
                    } else {
                        0
                    }
                }
            };
            return Some(profit);
        }
    }
    None
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
        let contract_id = env.register(TestContract, ());
        (env, contract_id)
    }

    // ── Validation: $0.10–$0.20, 10 levels ───────────────────────────────────
    // Prices are represented as integers; we use 1_000 = $0.10, 2_000 = $0.20.

    #[test]
    fn test_initialize_grid_success() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            assert_eq!(id, 0);

            let strategy = load(&env, id).unwrap();
            assert_eq!(strategy.grid_config.num_grids, 10);
            assert_eq!(strategy.grid_config.upper_price, 2_000);
            assert_eq!(strategy.grid_config.lower_price, 1_000);
            // spacing = (2000 - 1000) / (10 - 1) = 111
            assert_eq!(strategy.grid_config.grid_spacing, 111);
            assert_eq!(strategy.grid_config.order_size_per_grid, 1_000);
            assert_eq!(strategy.status, GridStatus::Initializing);
        });
    }

    #[test]
    fn test_initialize_invalid_price_range() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            let err =
                initialize_grid_strategy(&env, user, 1, 1_000, 2_000, 10, 10_000).unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidAmount);
        });
    }

    #[test]
    fn test_initialize_too_few_grids() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            let err = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 2, 10_000).unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidAmount);
        });
    }

    #[test]
    fn test_initialize_too_many_grids() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            let err =
                initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 51, 10_000).unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidAmount);
        });
    }

    #[test]
    fn test_place_grid_orders_count() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            // current price = 1_500 (mid-range) → 5 buys below, 4 sells above, skip exact mid
            set_mock_price(&env, 1, 1_500);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            let strategy = load(&env, id).unwrap();
            // levels: 1000,1111,1222,1333,1444 < 1500 → 5 buys
            //         1556,1667,1778,1889,2000 > 1500 → 5 sells  (no exact hit)
            assert_eq!(strategy.active_orders.len(), 10);
            assert_eq!(strategy.status, GridStatus::Active);
        });
    }

    #[test]
    fn test_place_grid_orders_skips_current_price_level() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            // Set price exactly at lower_price so level 0 is skipped
            set_mock_price(&env, 1, 1_000);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            let strategy = load(&env, id).unwrap();
            // level 0 (price=1000) skipped → 9 orders
            assert_eq!(strategy.active_orders.len(), 9);
        });
    }

    #[test]
    fn test_order_fill_and_rebalance() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            set_mock_price(&env, 1, 1_500);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            // Grab the order_id at level 0 (buy at 1000)
            let strategy = load(&env, id).unwrap();
            let order = strategy.active_orders.get(0u64).unwrap();
            let oid = order.order_id;
            let orders_before = strategy.active_orders.len();

            // Simulate fill
            on_grid_order_filled(&env, id, oid, 1_000, 1_000).unwrap();

            let strategy = load(&env, id).unwrap();
            // filled_orders grew by 1
            assert_eq!(strategy.filled_orders.len(), 1);
            // active_orders count unchanged (rebalance replaced it)
            assert_eq!(strategy.active_orders.len(), orders_before);
        });
    }

    #[test]
    fn test_profit_accumulates_on_oscillation() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            set_mock_price(&env, 1, 1_500);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            // Simulate sell fill at level 5 (price ≈ 1556)
            let strategy = load(&env, id).unwrap();
            let sell_order = strategy.active_orders.get(5u64).unwrap();
            let sell_oid = sell_order.order_id;
            on_grid_order_filled(&env, id, sell_oid, 1_556, 1_000).unwrap();

            // Now simulate buy fill at level 4 (adjacent, price ≈ 1444)
            let strategy = load(&env, id).unwrap();
            let buy_order = strategy.active_orders.get(4u64).unwrap();
            let buy_oid = buy_order.order_id;
            on_grid_order_filled(&env, id, buy_oid, 1_444, 1_000).unwrap();

            let strategy = load(&env, id).unwrap();
            // profit should be positive (sold high, bought low)
            assert!(strategy.total_profit > 0);
        });
    }

    #[test]
    fn test_adjust_grid_when_price_exits_range() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            set_mock_price(&env, 1, 1_500);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            // Price breaks above upper bound
            set_mock_price(&env, 1, 3_000);
            adjust_grid_to_price_movement(&env, id).unwrap();

            let strategy = load(&env, id).unwrap();
            // Grid re-centred: upper = 3000 + 500 = 3500, lower = 3000 - 500 = 2500
            assert_eq!(strategy.grid_config.upper_price, 3_500);
            assert_eq!(strategy.grid_config.lower_price, 2_500);
            assert_eq!(strategy.status, GridStatus::Active);
        });
    }

    #[test]
    fn test_adjust_grid_no_op_when_price_in_range() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            set_mock_price(&env, 1, 1_500);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            // Price stays inside range
            set_mock_price(&env, 1, 1_600);
            adjust_grid_to_price_movement(&env, id).unwrap();

            let strategy = load(&env, id).unwrap();
            // Bounds unchanged
            assert_eq!(strategy.grid_config.upper_price, 2_000);
            assert_eq!(strategy.grid_config.lower_price, 1_000);
        });
    }

    #[test]
    fn test_performance_metrics() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            set_mock_price(&env, 1, 1_500);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            let perf = calculate_grid_performance(&env, id).unwrap();
            assert_eq!(perf.total_profit, 0);
            assert_eq!(perf.total_fills, 0);
            assert_eq!(perf.active_orders, 10);
        });
    }

    #[test]
    fn test_fill_unknown_order_returns_error() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let user = Address::generate(&env);
            set_mock_price(&env, 1, 1_500);
            let id = initialize_grid_strategy(&env, user, 1, 2_000, 1_000, 10, 10_000).unwrap();
            place_grid_orders(&env, id).unwrap();

            let err = on_grid_order_filled(&env, id, 9_999_999, 1_000, 1_000).unwrap_err();
            assert_eq!(err, AutoTradeError::SignalNotFound);
        });
    }
}
