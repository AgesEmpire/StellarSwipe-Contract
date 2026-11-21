//! Portfolio drawdown trigger (Issue #674).
//!
//! Tracks a per-user high-water mark and fires a configurable action when the
//! current portfolio value drops more than a threshold below that mark.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Symbol};

use crate::errors::AutoTradeError;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Action taken when the drawdown threshold is breached.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawdownAction {
    /// Close all open positions for the user.
    ClosePositions,
    /// Pause copy-trading for the user.
    PauseCopying,
}

/// Per-user drawdown trigger configuration.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawdownTrigger {
    /// Enabled flag; when false no action fires.
    pub enabled: bool,
    /// Drawdown threshold in basis points (e.g. 1000 = 10 %).
    pub threshold_bps: u32,
    /// Action to execute when threshold is breached.
    pub action: DrawdownAction,
}

/// Per-user high-water mark state.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighWaterMark {
    /// The highest portfolio value (scaled ×10^7) observed for this user.
    pub peak_value: i128,
    /// Ledger timestamp of the last update.
    pub updated_at: u64,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
pub enum DrawdownKey {
    /// Per-user high-water mark.
    HighWaterMark(Address),
    /// Per-user drawdown trigger configuration.
    Trigger(Address),
}

// ── Storage helpers ───────────────────────────────────────────────────────────

pub fn get_high_water_mark(env: &Env, user: &Address) -> Option<HighWaterMark> {
    env.storage()
        .persistent()
        .get(&DrawdownKey::HighWaterMark(user.clone()))
}

pub fn set_high_water_mark(env: &Env, user: &Address, hwm: &HighWaterMark) {
    env.storage()
        .persistent()
        .set(&DrawdownKey::HighWaterMark(user.clone()), hwm);
}

pub fn get_drawdown_trigger(env: &Env, user: &Address) -> Option<DrawdownTrigger> {
    env.storage()
        .persistent()
        .get(&DrawdownKey::Trigger(user.clone()))
}

pub fn set_drawdown_trigger_config(env: &Env, user: &Address, trigger: &DrawdownTrigger) {
    env.storage()
        .persistent()
        .set(&DrawdownKey::Trigger(user.clone()), trigger);
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Configure the drawdown trigger for `user`.
pub fn configure_drawdown_trigger(
    env: &Env,
    user: &Address,
    threshold_bps: u32,
    action: DrawdownAction,
) -> Result<DrawdownTrigger, AutoTradeError> {
    if threshold_bps == 0 || threshold_bps > 10_000 {
        return Err(AutoTradeError::InvalidAmount);
    }
    let trigger = DrawdownTrigger {
        enabled: true,
        threshold_bps,
        action,
    };
    set_drawdown_trigger_config(env, user, &trigger);
    Ok(trigger)
}

/// Update the high-water mark with `current_value` and check whether the
/// drawdown trigger fires.
///
/// Returns `Some(action)` if the threshold was breached and the trigger is
/// enabled, `None` otherwise.
pub fn update_and_check(
    env: &Env,
    user: &Address,
    current_value: i128,
) -> Result<Option<DrawdownAction>, AutoTradeError> {
    if current_value < 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    let now = env.ledger().timestamp();
    let mut hwm = get_high_water_mark(env, user).unwrap_or(HighWaterMark {
        peak_value: current_value,
        updated_at: now,
    });

    if current_value > hwm.peak_value {
        hwm.peak_value = current_value;
        hwm.updated_at = now;
        set_high_water_mark(env, user, &hwm);
        env.events().publish(
            (Symbol::new(env, "hwm_updated"), user.clone()),
            (hwm.peak_value, now),
        );
        return Ok(None);
    }

    set_high_water_mark(env, user, &hwm);

    let trigger = match get_drawdown_trigger(env, user) {
        Some(t) if t.enabled => t,
        _ => return Ok(None),
    };

    if is_drawdown_breached(hwm.peak_value, current_value, trigger.threshold_bps) {
        env.events().publish(
            (Symbol::new(env, "drawdown_triggered"), user.clone()),
            (hwm.peak_value, current_value, trigger.threshold_bps, now),
        );
        return Ok(Some(trigger.action));
    }

    Ok(None)
}

/// Pure check: is `current_value` below `peak_value * (1 - threshold_bps/10000)`?
pub fn is_drawdown_breached(peak_value: i128, current_value: i128, threshold_bps: u32) -> bool {
    if peak_value <= 0 {
        return false;
    }
    // current_value * 10000 < peak_value * (10000 - threshold_bps)
    let lhs = current_value.saturating_mul(10_000);
    let rhs = peak_value.saturating_mul((10_000i128 - threshold_bps as i128).max(0));
    lhs < rhs
}

/// Check if the drawdown trigger fires for `user` at `current_value` without
/// updating the high-water mark (read-only evaluation).
pub fn check_drawdown_trigger(env: &Env, user: &Address, current_value: i128) -> bool {
    let trigger = match get_drawdown_trigger(env, user) {
        Some(t) if t.enabled => t,
        _ => return false,
    };
    match get_high_water_mark(env, user) {
        Some(hwm) => is_drawdown_breached(hwm.peak_value, current_value, trigger.threshold_bps),
        None => false,
    }
}
