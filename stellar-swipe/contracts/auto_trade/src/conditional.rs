//! Options-Style Conditional Orders
//!
//! Supports complex trigger logic (AND/OR) combining price, time, and technical
//! conditions — mimicking options strategies without actual options.

#![allow(dead_code)]

use crate::errors::AutoTradeError;
use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Direction of a price move condition.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceDirection {
    Above,
    Below,
}

/// A single atomic trigger condition.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    /// Price of `.0` is `.1` `.2` (scaled ×10^7).
    Price(u32, PriceDirection, i128),
    /// Current ledger timestamp ≥ inner value.
    TimeAfter(u64),
    /// Price dropped by `.1` bps from peak, rebounded by `.2` bps from trough (asset `.0`).
    PriceDropRebound(u32, u32, u32),
    /// Volatility breakout: asset `.0`, threshold `.1` bps from reference.
    VolatilityBreakout(u32, u32),
}

/// How multiple conditions are combined.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
}

/// Side of the conditional order.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionalSide {
    Buy,
    Sell,
}

/// Lifecycle status of a conditional order.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionalStatus {
    Pending,
    Triggered,
    Executed,
    Expired,
    Cancelled,
}

/// A conditional order that executes when its trigger logic fires.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ConditionalOrder {
    pub id: u64,
    pub user: Address,
    pub asset_id: u32,
    pub side: ConditionalSide,
    pub amount: i128,
    /// Limit price for execution (0 = market).
    pub limit_price: i128,
    pub conditions: Vec<Condition>,
    pub logic: LogicOp,
    pub status: ConditionalStatus,
    pub created_at: u64,
    pub expires_at: u64,
    /// Price of `asset_id` at order creation — used by breakout / rebound checks.
    pub reference_price: i128,
    /// Lowest price seen since creation — used by rebound check.
    pub trough_price: i128,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
pub enum ConditionalKey {
    Counter,
    Order(u64),
    ActiveOrders,
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn next_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .persistent()
        .get(&ConditionalKey::Counter)
        .unwrap_or(0)
        + 1;
    env.storage()
        .persistent()
        .set(&ConditionalKey::Counter, &id);
    id
}

fn save(env: &Env, order: &ConditionalOrder) {
    env.storage()
        .persistent()
        .set(&ConditionalKey::Order(order.id), order);
}

fn load(env: &Env, id: u64) -> Result<ConditionalOrder, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&ConditionalKey::Order(id))
        .ok_or(AutoTradeError::ConditionalOrderNotFound)
}

fn active_ids(env: &Env) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&ConditionalKey::ActiveOrders)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_active_ids(env: &Env, ids: &Vec<u64>) {
    env.storage()
        .persistent()
        .set(&ConditionalKey::ActiveOrders, ids);
}

fn add_active(env: &Env, id: u64) {
    let mut ids = active_ids(env);
    if !ids.contains(id) {
        ids.push_back(id);
        set_active_ids(env, &ids);
    }
}

fn remove_active(env: &Env, id: u64) {
    let mut ids = active_ids(env);
    if let Some(pos) = ids.first_index_of(id) {
        ids.remove(pos);
        set_active_ids(env, &ids);
    }
}

// ── Condition evaluation ──────────────────────────────────────────────────────

/// Returns the current price for `asset_id` from the risk module's price
/// store, or `None` when no price has ever been recorded for this asset.
///
/// This goes through `risk::get_asset_price`, the same read path used by the
/// rest of the auto-trade contract, so it reflects the price actually
/// written by `risk::set_asset_price` (temporary storage) via oracle pushes
/// and manual price updates. Previously this read directly from a different
/// storage durability tier (`persistent`) than production ever wrote to
/// (`temporary`), so in real usage a price was never found here and this
/// silently defaulted to `0` — see `eval_condition` for why that is unsafe.
fn current_price(env: &Env, asset_id: u32) -> Option<i128> {
    crate::risk::get_asset_price(env, asset_id)
}

fn eval_condition(env: &Env, cond: &Condition, order: &ConditionalOrder) -> bool {
    match cond {
        Condition::Price(asset_id, direction, threshold) => {
            // Missing price data must never be treated as `0` — a `Below`
            // condition would then trigger immediately for any positive
            // threshold, firing the order on garbage data instead of a real
            // price. Fail safe: no price means the condition is not met.
            let Some(price) = current_price(env, *asset_id) else {
                return false;
            };
            match direction {
                PriceDirection::Above => price >= *threshold,
                PriceDirection::Below => price <= *threshold,
            }
        }
        Condition::TimeAfter(after_ts) => env.ledger().timestamp() >= *after_ts,
        Condition::PriceDropRebound(asset_id, drop_bps, rebound_bps) => {
            let Some(price) = current_price(env, *asset_id) else {
                return false;
            };
            let ref_price = order.reference_price;
            if ref_price == 0 {
                return false;
            }
            // Drop threshold: ref_price * (10000 - drop_bps) / 10000
            let drop_threshold = ref_price * (10_000 - *drop_bps as i128) / 10_000;
            let trough = order.trough_price;
            if trough == 0 || trough > drop_threshold {
                return false;
            }
            // Rebound: price ≥ trough * (10000 + rebound_bps) / 10000
            let rebound_threshold = trough * (10_000 + *rebound_bps as i128) / 10_000;
            price >= rebound_threshold
        }
        Condition::VolatilityBreakout(asset_id, threshold_bps) => {
            let Some(price) = current_price(env, *asset_id) else {
                return false;
            };
            let ref_price = order.reference_price;
            if ref_price == 0 {
                return false;
            }
            let diff = if price > ref_price {
                price - ref_price
            } else {
                ref_price - price
            };
            diff * 10_000 >= ref_price * *threshold_bps as i128
        }
    }
}

fn all_triggered(env: &Env, order: &ConditionalOrder) -> bool {
    match order.logic {
        LogicOp::And => {
            for i in 0..order.conditions.len() {
                if !eval_condition(env, &order.conditions.get(i).unwrap(), order) {
                    return false;
                }
            }
            true
        }
        LogicOp::Or => {
            for i in 0..order.conditions.len() {
                if eval_condition(env, &order.conditions.get(i).unwrap(), order) {
                    return true;
                }
            }
            false
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Create a new conditional order.
pub fn create_conditional_order(
    env: &Env,
    user: Address,
    asset_id: u32,
    side: ConditionalSide,
    amount: i128,
    limit_price: i128,
    conditions: Vec<Condition>,
    logic: LogicOp,
    expires_in_seconds: u64,
) -> Result<u64, AutoTradeError> {
    user.require_auth();

    if amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    if conditions.is_empty() {
        return Err(AutoTradeError::InvalidConditionalConfig);
    }

    let now = env.ledger().timestamp();
    // 0 is the existing "no reference price" sentinel used by the
    // PriceDropRebound / VolatilityBreakout checks above, so a missing price
    // here correctly disables those checks rather than seeding them with 0.
    let ref_price = current_price(env, asset_id).unwrap_or(0);
    let id = next_id(env);

    let order = ConditionalOrder {
        id,
        user: user.clone(),
        asset_id,
        side,
        amount,
        limit_price,
        conditions,
        logic,
        status: ConditionalStatus::Pending,
        created_at: now,
        expires_at: now + expires_in_seconds,
        reference_price: ref_price,
        trough_price: ref_price,
    };

    save(env, &order);
    add_active(env, id);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "cond_order_created"), user, id),
        (asset_id, amount),
    );

    Ok(id)
}

/// Cancel a pending conditional order (owner only).
pub fn cancel_conditional_order(env: &Env, id: u64, user: Address) -> Result<(), AutoTradeError> {
    user.require_auth();
    let mut order = load(env, id)?;
    if order.user != user {
        return Err(AutoTradeError::Unauthorized);
    }
    if order.status != ConditionalStatus::Pending {
        return Err(AutoTradeError::ConditionalOrderNotPending);
    }
    order.status = ConditionalStatus::Cancelled;
    save(env, &order);
    remove_active(env, id);

    #[allow(deprecated)]
    env.events()
        .publish((Symbol::new(env, "cond_order_cancelled"), user, id), ());

    Ok(())
}

/// Get a conditional order by id.
pub fn get_conditional_order(env: &Env, id: u64) -> Result<ConditionalOrder, AutoTradeError> {
    load(env, id)
}

/// Process all active conditional orders against current market prices.
/// Returns the ids of orders that were triggered (and marked Triggered).
/// Call `execute_triggered_orders` afterwards to actually fill them.
pub fn check_and_trigger(env: &Env) -> Vec<u64> {
    let now = env.ledger().timestamp();
    let ids = active_ids(env);
    let mut triggered = Vec::new(env);

    for i in 0..ids.len() {
        let id = ids.get(i).unwrap();
        let mut order = match load(env, id) {
            Ok(o) => o,
            Err(_) => continue,
        };

        if order.status != ConditionalStatus::Pending {
            remove_active(env, id);
            continue;
        }

        // Expire stale orders
        if now >= order.expires_at {
            order.status = ConditionalStatus::Expired;
            save(env, &order);
            remove_active(env, id);
            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(env, "cond_order_expired"),
                    order.user.clone(),
                    id,
                ),
                (),
            );
            continue;
        }

        // Update trough for rebound tracking. No price available (asset was
        // never priced, or the price expired) means there is nothing to
        // record — leave the existing trough untouched.
        if let Some(price) = current_price(env, order.asset_id) {
            if price > 0 && (order.trough_price == 0 || price < order.trough_price) {
                order.trough_price = price;
            }
        }

        if all_triggered(env, &order) {
            order.status = ConditionalStatus::Triggered;
            save(env, &order);
            remove_active(env, id);
            triggered.push_back(id);

            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(env, "cond_order_triggered"),
                    order.user.clone(),
                    id,
                ),
                (order.asset_id, order.amount),
            );
        } else {
            // Persist updated trough
            save(env, &order);
        }
    }

    triggered
}

/// Mark a triggered order as Executed (called after the trade is filled).
pub fn mark_executed(env: &Env, id: u64) -> Result<(), AutoTradeError> {
    let mut order = load(env, id)?;
    if order.status != ConditionalStatus::Triggered {
        return Err(AutoTradeError::ConditionalOrderNotTriggered);
    }
    order.status = ConditionalStatus::Executed;
    save(env, &order);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "cond_order_executed"),
            order.user.clone(),
            id,
        ),
        (order.asset_id, order.amount),
    );

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AutoTradeContract, AutoTradeContractClient};
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address, Env,
    };

    fn setup() -> (Env, Address, AutoTradeContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let contract = env.register(AutoTradeContract, ());
        let client = AutoTradeContractClient::new(&env, &contract);
        (env, contract, client)
    }

    /// Registers an admin + a keeper address on `contract`, returning the keeper.
    /// Needed because `check_and_trigger_conditionals` now requires a registered,
    /// authenticated keeper caller.
    fn setup_keeper(env: &Env, contract: &Address) -> Address {
        let admin = Address::generate(env);
        let keeper = Address::generate(env);
        env.as_contract(contract, || {
            crate::admin::init_admin(env, admin.clone());
            crate::keeper::add_keeper(env, &admin, keeper.clone()).unwrap();
        });
        keeper
    }

    /// Seeds a price via the same path production code uses
    /// (`risk::set_asset_price`), so these tests exercise the real read/write
    /// path instead of poking storage that `current_price` never actually
    /// reads in production.
    fn seed_price(env: &Env, contract: &Address, asset_id: u32, price: i128) {
        env.as_contract(contract, || {
            crate::risk::set_asset_price(env, asset_id, price);
        });
    }

    fn simple_price_condition(
        env: &Env,
        asset_id: u32,
        direction: PriceDirection,
        threshold: i128,
    ) -> Vec<Condition> {
        let mut v = Vec::new(env);
        v.push_back(Condition::Price(asset_id, direction, threshold));
        v
    }

    #[test]
    fn test_create_and_get() {
        let (env, contract, client) = setup();
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );
        let order = client.get_conditional_order(&id);
        assert_eq!(order.status, ConditionalStatus::Pending);
        assert_eq!(order.reference_price, 100_000);
    }

    #[test]
    fn test_cancel_order() {
        let (env, contract, client) = setup();
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );
        client.cancel_conditional_order(&id, &user);
        let order = client.get_conditional_order(&id);
        assert_eq!(order.status, ConditionalStatus::Cancelled);
    }

    #[test]
    fn test_cancel_wrong_user_fails() {
        let (env, contract, client) = setup();
        let user = Address::generate(&env);
        let other = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );
        assert_eq!(
            client.try_cancel_conditional_order(&id, &other),
            Err(Ok(AutoTradeError::Unauthorized))
        );
    }

    #[test]
    fn test_price_above_triggers() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );

        assert_eq!(client.check_and_trigger_conditionals(&keeper).len(), 0);

        seed_price(&env, &contract, 1, 115_000);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
        assert_eq!(
            client.get_conditional_order(&id).status,
            ConditionalStatus::Triggered
        );
    }

    #[test]
    fn test_price_below_triggers() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Below, 90_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Sell,
            &500,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );

        seed_price(&env, &contract, 1, 85_000);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    /// Missing price data (asset never priced by any oracle/whitelisted
    /// source) must never be silently treated as price `0`. A `Below`
    /// condition would otherwise be satisfied by any positive threshold and
    /// fire an order against garbage data instead of a real price.
    ///
    /// Guards against the bug where `current_price` read from a storage
    /// durability tier (`persistent`) that production price updates
    /// (`risk::set_asset_price`) never actually write to (`temporary`) — so
    /// in real usage a price was never found and defaulted to `0`.
    #[test]
    fn test_price_condition_does_not_trigger_on_missing_price() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);

        // No seed_price call for asset 2 — its price has never been recorded.
        let conditions = simple_price_condition(&env, 2, PriceDirection::Below, 90_000);
        let id = client.create_conditional_order(
            &user,
            &2,
            &ConditionalSide::Sell,
            &500,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );

        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(
            triggered.len(),
            0,
            "an order must not trigger on an asset with no recorded price"
        );
        assert_eq!(
            client.get_conditional_order(&id).status,
            ConditionalStatus::Pending
        );
    }

    #[test]
    fn test_time_after_triggers() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::TimeAfter(2_000));
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &10_000,
        );

        assert_eq!(client.check_and_trigger_conditionals(&keeper).len(), 0);

        env.ledger().set_timestamp(2_001);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    #[test]
    fn test_drop_rebound_triggers() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::PriceDropRebound(1, 1_000, 300));
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &10_000,
        );

        seed_price(&env, &contract, 1, 89_000);
        assert_eq!(client.check_and_trigger_conditionals(&keeper).len(), 0);

        seed_price(&env, &contract, 1, 91_700);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    #[test]
    fn test_volatility_breakout_triggers() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::VolatilityBreakout(1, 500));
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &10_000,
        );

        seed_price(&env, &contract, 1, 104_000);
        assert_eq!(client.check_and_trigger_conditionals(&keeper).len(), 0);

        seed_price(&env, &contract, 1, 106_000);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    #[test]
    fn test_and_logic_requires_all() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        seed_price(&env, &contract, 2, 50_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::Price(1, PriceDirection::Above, 110_000));
        conditions.push_back(Condition::Price(2, PriceDirection::Below, 40_000));
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &10_000,
        );

        seed_price(&env, &contract, 1, 115_000);
        assert_eq!(client.check_and_trigger_conditionals(&keeper).len(), 0);

        seed_price(&env, &contract, 2, 35_000);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    #[test]
    fn test_or_logic_requires_one() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        seed_price(&env, &contract, 2, 50_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::Price(1, PriceDirection::Above, 110_000));
        conditions.push_back(Condition::Price(2, PriceDirection::Below, 40_000));
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::Or,
            &10_000,
        );

        seed_price(&env, &contract, 1, 115_000);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    #[test]
    fn test_order_expires() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 200_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &500,
        );

        env.ledger().set_timestamp(1_600);
        let triggered = client.check_and_trigger_conditionals(&keeper);
        assert_eq!(triggered.len(), 0);
        assert_eq!(
            client.get_conditional_order(&id).status,
            ConditionalStatus::Expired
        );
    }

    #[test]
    fn test_mark_executed() {
        let (env, contract, client) = setup();
        let keeper = setup_keeper(&env, &contract);
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 120_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );

        client.check_and_trigger_conditionals(&keeper);
        assert_eq!(
            client.get_conditional_order(&id).status,
            ConditionalStatus::Triggered
        );

        client.mark_conditional_executed(&id);
        assert_eq!(
            client.get_conditional_order(&id).status,
            ConditionalStatus::Executed
        );
    }

    #[test]
    fn test_mark_executed_wrong_state_fails() {
        let (env, contract, client) = setup();
        let user = Address::generate(&env);
        seed_price(&env, &contract, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = client.create_conditional_order(
            &user,
            &1,
            &ConditionalSide::Buy,
            &1_000,
            &0,
            &conditions,
            &LogicOp::And,
            &3_600,
        );
        assert_eq!(
            client.try_mark_conditional_executed(&id),
            Err(Ok(AutoTradeError::ConditionalOrderNotTriggered))
        );
    }
}
