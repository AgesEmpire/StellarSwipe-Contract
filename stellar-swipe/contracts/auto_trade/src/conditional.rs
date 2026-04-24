//! Options-Style Conditional Orders
//!
//! Supports complex trigger logic (AND/OR) combining price, time, and technical
//! conditions — mimicking options strategies without actual options.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};
use crate::errors::AutoTradeError;

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
    let id: u64 = env.storage().persistent().get(&ConditionalKey::Counter).unwrap_or(0) + 1;
    env.storage().persistent().set(&ConditionalKey::Counter, &id);
    id
}

fn save(env: &Env, order: &ConditionalOrder) {
    env.storage().persistent().set(&ConditionalKey::Order(order.id), order);
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
    env.storage().persistent().set(&ConditionalKey::ActiveOrders, ids);
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

/// Returns the current price for `asset_id` from the risk module's price store.
fn current_price(env: &Env, asset_id: u32) -> i128 {
    use crate::risk::RiskDataKey;
    env.storage()
        .persistent()
        .get(&RiskDataKey::AssetPrice(asset_id))
        .unwrap_or(0)
}

fn eval_condition(env: &Env, cond: &Condition, order: &ConditionalOrder) -> bool {
    match cond {
        Condition::Price(asset_id, direction, threshold) => {
            let price = current_price(env, *asset_id);
            match direction {
                PriceDirection::Above => price >= *threshold,
                PriceDirection::Below => price <= *threshold,
            }
        }
        Condition::TimeAfter(after_ts) => env.ledger().timestamp() >= *after_ts,
        Condition::PriceDropRebound(asset_id, drop_bps, rebound_bps) => {
            let price = current_price(env, *asset_id);
            let ref_price = order.reference_price;
            if ref_price == 0 { return false; }
            // Drop threshold: ref_price * (10000 - drop_bps) / 10000
            let drop_threshold = ref_price * (10_000 - *drop_bps as i128) / 10_000;
            let trough = order.trough_price;
            if trough == 0 || trough > drop_threshold { return false; }
            // Rebound: price ≥ trough * (10000 + rebound_bps) / 10000
            let rebound_threshold = trough * (10_000 + *rebound_bps as i128) / 10_000;
            price >= rebound_threshold
        }
        Condition::VolatilityBreakout(asset_id, threshold_bps) => {
            let price = current_price(env, *asset_id);
            let ref_price = order.reference_price;
            if ref_price == 0 { return false; }
            let diff = if price > ref_price { price - ref_price } else { ref_price - price };
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
    let ref_price = current_price(env, asset_id);
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
    env.events().publish(
        (Symbol::new(env, "cond_order_cancelled"), user, id),
        (),
    );

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
                (Symbol::new(env, "cond_order_expired"), order.user.clone(), id),
                (),
            );
            continue;
        }

        // Update trough for rebound tracking
        let price = current_price(env, order.asset_id);
        if price > 0 && (order.trough_price == 0 || price < order.trough_price) {
            order.trough_price = price;
        }

        if all_triggered(env, &order) {
            order.status = ConditionalStatus::Triggered;
            save(env, &order);
            remove_active(env, id);
            triggered.push_back(id);

            #[allow(deprecated)]
            env.events().publish(
                (Symbol::new(env, "cond_order_triggered"), order.user.clone(), id),
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
        (Symbol::new(env, "cond_order_executed"), order.user.clone(), id),
        (order.asset_id, order.amount),
    );

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::RiskDataKey;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env,
    };

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let user = Address::generate(&env);
        (env, user)
    }

    fn set_price(env: &Env, asset_id: u32, price: i128) {
        env.storage()
            .persistent()
            .set(&RiskDataKey::AssetPrice(asset_id), &price);
    }

    fn simple_price_condition(env: &Env, asset_id: u32, direction: PriceDirection, threshold: i128) -> Vec<Condition> {
        let mut v = Vec::new(env);
        v.push_back(Condition::Price(asset_id, direction, threshold));
        v
    }

    // ── create / cancel ───────────────────────────────────────────────────────

    #[test]
    fn test_create_and_get() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 3_600).unwrap();
        let order = get_conditional_order(&env, id).unwrap();
        assert_eq!(order.status, ConditionalStatus::Pending);
        assert_eq!(order.reference_price, 100_000);
    }

    #[test]
    fn test_cancel_order() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 3_600).unwrap();
        cancel_conditional_order(&env, id, user).unwrap();
        let order = get_conditional_order(&env, id).unwrap();
        assert_eq!(order.status, ConditionalStatus::Cancelled);
    }

    #[test]
    fn test_cancel_wrong_user_fails() {
        let (env, user) = setup();
        let other = Address::generate(&env);
        set_price(&env, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 3_600).unwrap();
        assert_eq!(cancel_conditional_order(&env, id, other), Err(AutoTradeError::Unauthorized));
    }

    // ── price trigger ─────────────────────────────────────────────────────────

    #[test]
    fn test_price_above_triggers() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 3_600).unwrap();

        // Price not yet above threshold
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 0);

        // Price crosses threshold
        set_price(&env, 1, 115_000);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
        assert_eq!(get_conditional_order(&env, id).unwrap().status, ConditionalStatus::Triggered);
    }

    #[test]
    fn test_price_below_triggers() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Below, 90_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Sell, 500, 0, conditions, LogicOp::And, 3_600).unwrap();

        set_price(&env, 1, 85_000);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    // ── time trigger ──────────────────────────────────────────────────────────

    #[test]
    fn test_time_after_triggers() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::TimeAfter(2_000));
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 10_000).unwrap();

        // Time not yet reached
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 0);

        env.ledger().set_timestamp(2_001);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    // ── drop-rebound trigger ──────────────────────────────────────────────────

    #[test]
    fn test_drop_rebound_triggers() {
        let (env, user) = setup();
        // Reference price = 100_000
        set_price(&env, 1, 100_000);
        let mut conditions = Vec::new(&env);
        // Drop 10% then rebound 3%
        conditions.push_back(Condition::PriceDropRebound(1, 1_000, 300));
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 10_000).unwrap();

        // Price drops to 89_000 (< 90_000 threshold) — trough updated, no rebound yet
        set_price(&env, 1, 89_000);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 0);

        // Price rebounds to 89_000 * 1.03 = 91_670 — should trigger
        set_price(&env, 1, 91_700);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    // ── volatility breakout ───────────────────────────────────────────────────

    #[test]
    fn test_volatility_breakout_triggers() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let mut conditions = Vec::new(&env);
        // 5% breakout = 500 bps
        conditions.push_back(Condition::VolatilityBreakout(1, 500));
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 10_000).unwrap();

        set_price(&env, 1, 104_000); // only 4% — not enough
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 0);

        set_price(&env, 1, 106_000); // 6% — triggers
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    // ── AND / OR logic ────────────────────────────────────────────────────────

    #[test]
    fn test_and_logic_requires_all() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        set_price(&env, 2, 50_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::Price(1, PriceDirection::Above, 110_000));
        conditions.push_back(Condition::Price(2, PriceDirection::Below, 40_000));
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 10_000).unwrap();

        // Only first condition met
        set_price(&env, 1, 115_000);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 0);

        // Both conditions met
        set_price(&env, 2, 35_000);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    #[test]
    fn test_or_logic_requires_one() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        set_price(&env, 2, 50_000);
        let mut conditions = Vec::new(&env);
        conditions.push_back(Condition::Price(1, PriceDirection::Above, 110_000));
        conditions.push_back(Condition::Price(2, PriceDirection::Below, 40_000));
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::Or, 10_000).unwrap();

        // Only first condition met — OR should trigger
        set_price(&env, 1, 115_000);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered.get(0).unwrap(), id);
    }

    // ── expiry ────────────────────────────────────────────────────────────────

    #[test]
    fn test_order_expires() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 200_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 500).unwrap();

        // Advance past expiry (1_000 + 500 = 1_500)
        env.ledger().set_timestamp(1_600);
        let triggered = check_and_trigger(&env);
        assert_eq!(triggered.len(), 0);
        assert_eq!(get_conditional_order(&env, id).unwrap().status, ConditionalStatus::Expired);
    }

    // ── mark_executed ─────────────────────────────────────────────────────────

    #[test]
    fn test_mark_executed() {
        let (env, user) = setup();
        set_price(&env, 1, 120_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 3_600).unwrap();

        check_and_trigger(&env);
        assert_eq!(get_conditional_order(&env, id).unwrap().status, ConditionalStatus::Triggered);

        mark_executed(&env, id).unwrap();
        assert_eq!(get_conditional_order(&env, id).unwrap().status, ConditionalStatus::Executed);
    }

    #[test]
    fn test_mark_executed_wrong_state_fails() {
        let (env, user) = setup();
        set_price(&env, 1, 100_000);
        let conditions = simple_price_condition(&env, 1, PriceDirection::Above, 110_000);
        let id = create_conditional_order(&env, user.clone(), 1, ConditionalSide::Buy, 1_000, 0, conditions, LogicOp::And, 3_600).unwrap();
        // Still Pending — should fail
        assert_eq!(mark_executed(&env, id), Err(AutoTradeError::ConditionalOrderNotTriggered));
    }
}
