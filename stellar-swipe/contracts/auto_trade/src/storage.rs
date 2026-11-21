#![allow(dead_code)]
use soroban_sdk::{contracttype, symbol_short, Address, Env};
use stellar_swipe_common::storage_crud::{
    crud_get, crud_get_or, crud_has, crud_remove, crud_set, StorageTier,
};

use crate::auth::{AuthConfig, AuthKey};

#[contracttype]
#[derive(Clone)]
pub struct Signal {
    pub signal_id: u64,
    pub price: i128,
    pub expiry: u64,
    pub base_asset: u32,
}

/// Issue #1438: cumulative fill-tracking for an auto-trade "intent" (the
/// combination of a user and a signal they are trading against). A single
/// `execute_trade` call may only partially fill the user's desired amount;
/// this record lets subsequent calls top up the fill instead of either
/// silently dropping the unfilled remainder or double-counting against the
/// originally requested amount.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentFill {
    pub user: Address,
    pub signal_id: u64,
    /// Total amount the user has asked to fill for this intent across all
    /// `execute_trade` calls so far (grows if the user raises their ask).
    pub total_requested: i128,
    /// Total amount actually filled so far across all partial executions.
    pub total_filled: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimitInfo {
    pub user: Address,
    pub is_limited: bool,
    pub expires_at: u64,
}

#[contracttype]
pub enum DataKey {
    Trades(Address, u64),
    Signal(u64),
    SignalStatus(u64),
}

/// Lifecycle status of a signal, checked at execution time (#1365).
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalStatus {
    Active,
    Archived,
    Invalidated,
}

/// Current signal status; signals with no recorded status are `Active`.
pub fn get_signal_status(env: &Env, id: u64) -> SignalStatus {
    env.storage()
        .persistent()
        .get(&DataKey::SignalStatus(id))
        .unwrap_or(SignalStatus::Active)
}

pub fn set_signal_status(env: &Env, id: u64, status: SignalStatus) {
    env.storage().persistent().set(&DataKey::SignalStatus(id), &status);
    RateLimitInfo(Address),
    /// Last ledger timestamp at which `(user, signal_id)` executed a trade
    /// (per-signal execution rate limit).
    LastExecution(Address, u64),
    /// Issue #1438: cumulative fill state for a (user, signal) intent.
    IntentFill(Address, u64),
}

/// Get the cumulative fill record for a (user, signal_id) intent, if any.
pub fn get_intent_fill(env: &Env, user: &Address, signal_id: u64) -> Option<IntentFill> {
    crud_get(
        env,
        StorageTier::Persistent,
        &DataKey::IntentFill(user.clone(), signal_id),
    )
}

/// Persist the cumulative fill record for a (user, signal_id) intent.
pub fn set_intent_fill(env: &Env, fill: &IntentFill) {
    crud_set(
        env,
        StorageTier::Persistent,
        &DataKey::IntentFill(fill.user.clone(), fill.signal_id),
        fill,
    );
}

/// Remaining amount still to be filled for a (user, signal_id) intent,
/// given a freshly requested `amount`. If the user has already partially
/// filled this intent, the remaining amount is capped so the cumulative
/// filled amount never exceeds the larger of the prior requested amount or
/// the new request (allowing the user to top up their ask).
pub fn remaining_intent_amount(env: &Env, user: &Address, signal_id: u64, amount: i128) -> i128 {
    match get_intent_fill(env, user, signal_id) {
        Some(fill) => {
            let total_requested = if amount > fill.total_requested {
                amount
            } else {
                fill.total_requested
            };
            let remaining = total_requested - fill.total_filled;
            if remaining < 0 {
                0
            } else {
                remaining
            }
        }
        None => amount,
    }
}

/// Record that `newly_filled` additional units were filled for a
/// (user, signal_id) intent whose latest request was `requested_amount`.
pub fn record_intent_fill(
    env: &Env,
    user: &Address,
    signal_id: u64,
    requested_amount: i128,
    newly_filled: i128,
) {
    let mut fill = get_intent_fill(env, user, signal_id).unwrap_or(IntentFill {
        user: user.clone(),
        signal_id,
        total_requested: 0,
        total_filled: 0,
    });
    if requested_amount > fill.total_requested {
        fill.total_requested = requested_amount;
    }
    fill.total_filled += newly_filled;
    set_intent_fill(env, &fill);
}

/// Get a signal by ID
pub fn get_signal(env: &Env, id: u64) -> Option<Signal> {
    crud_get(env, StorageTier::Persistent, &DataKey::Signal(id))
}

/// Set a signal
pub fn set_signal(env: &Env, id: u64, signal: &Signal) {
    crud_set(env, StorageTier::Persistent, &DataKey::Signal(id), signal);
}

/// Test helper: auth plus max temporary SDEX balance.
pub fn authorize_user(env: &Env, user: &Address) {
    authorize_user_with_limits(env, user, i128::MAX / 4, 30);
    env.storage()
        .temporary()
        .set(&(user.clone(), symbol_short!("balance")), &i128::MAX);
}

/// Authorize a user with explicit limits.
pub fn authorize_user_with_limits(
    env: &Env,
    user: &Address,
    max_trade_amount: i128,
    duration_days: u32,
) {
    let config = AuthConfig {
        authorized: true,
        max_trade_amount,
        expires_at: env.ledger().timestamp() + (duration_days as u64 * 86400),
        granted_at: env.ledger().timestamp(),
    };
    crud_set(
        env,
        StorageTier::Persistent,
        &AuthKey::Authorization(user.clone()),
        &config,
    );
    crud_set(
        env,
        StorageTier::Temporary,
        &(user.clone(), symbol_short!("balance")),
        &i128::MAX,
    );
}

pub fn revoke_user_authorization(env: &Env, user: &Address) {
    crud_remove(
        env,
        StorageTier::Persistent,
        &AuthKey::Authorization(user.clone()),
    );
}

/// Get the stored rate-limit info for a user, if any.
pub fn get_rate_limit_info(env: &Env, user: &Address) -> Option<RateLimitInfo> {
    crud_get(
        env,
        StorageTier::Persistent,
        &DataKey::RateLimitInfo(user.clone()),
    )
}

/// Persist rate-limit info for a user.
pub fn set_rate_limit_info(env: &Env, user: &Address, info: &RateLimitInfo) {
    crud_set(
        env,
        StorageTier::Persistent,
        &DataKey::RateLimitInfo(user.clone()),
        info,
    );
}

/// Whether a user is currently rate limited (flag set and not yet expired).
pub fn is_rate_limited(env: &Env, user: &Address) -> bool {
    match get_rate_limit_info(env, user) {
        Some(info) => info.is_limited && env.ledger().timestamp() < info.expires_at,
        None => false,
    }
}

// ── Loss-streak pause (Issue #698) ────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LossStreakCounter {
    pub consecutive_losses: u32,
    pub updated_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossStreakConfig {
    /// Number of consecutive losing auto-trades that trigger an automatic pause.
    pub threshold: u32,
}

impl Default for LossStreakConfig {
    fn default() -> Self {
        Self { threshold: 5 }
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LossStreakKey {
    /// Per-user consecutive-loss counter. Deliberately NOT named `Counter` to
    /// avoid a storage-key collision with `DailyExecutionCapKey::Counter`.
    StreakCounter(Address),
    ConfigState,
    /// Whether auto-trading is paused due to loss-streak for a user.
    Paused(Address),
}

/// Get the per-user consecutive-loss counter.
pub fn get_loss_streak_counter(env: &Env, user: &Address) -> LossStreakCounter {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &LossStreakKey::StreakCounter(user.clone()),
        LossStreakCounter::default(),
    )
}

/// Set the per-user consecutive-loss counter.
pub fn set_loss_streak_counter(env: &Env, user: &Address, counter: &LossStreakCounter) {
    crud_set(
        env,
        StorageTier::Persistent,
        &LossStreakKey::StreakCounter(user.clone()),
        counter,
    );
}

/// Get the loss-streak threshold config.
pub fn get_loss_streak_config(env: &Env) -> LossStreakConfig {
    crud_get_or(
        env,
        StorageTier::Instance,
        &LossStreakKey::ConfigState,
        LossStreakConfig::default(),
    )
}

/// Set the loss-streak threshold config (admin only).
pub fn set_loss_streak_config(env: &Env, config: &LossStreakConfig) {
    crud_set(
        env,
        StorageTier::Instance,
        &LossStreakKey::ConfigState,
        config,
    );
}

/// Whether the user is currently paused due to a loss-streak.
pub fn is_loss_streak_paused(env: &Env, user: &Address) -> bool {
    crud_has(
        env,
        StorageTier::Persistent,
        &LossStreakKey::Paused(user.clone()),
    )
}

/// Mark a user as paused due to loss-streak.
pub fn set_loss_streak_paused(env: &Env, user: &Address) {
    crud_set(
        env,
        StorageTier::Persistent,
        &LossStreakKey::Paused(user.clone()),
        &true,
    );
}

/// Clear the loss-streak pause for a user.
pub fn clear_loss_streak_paused(env: &Env, user: &Address) {
    crud_remove(
        env,
        StorageTier::Persistent,
        &LossStreakKey::Paused(user.clone()),
    );
}

// ── Daily Execution Cap ───────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DailyExecutionCounter {
    pub count: u32,
    pub window_start: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailyCapConfig {
    /// Maximum auto-trade executions allowed per rolling 24-hour window.
    pub max_executions: u32,
}

impl Default for DailyCapConfig {
    fn default() -> Self {
        Self { max_executions: 10 }
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DailyExecutionCapKey {
    Counter(Address),
    Config(Address),
}

pub fn get_daily_execution_counter(env: &Env, user: &Address) -> DailyExecutionCounter {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &DailyExecutionCapKey::Counter(user.clone()),
        DailyExecutionCounter::default(),
    )
}

pub fn set_daily_execution_counter(env: &Env, user: &Address, counter: &DailyExecutionCounter) {
    crud_set(
        env,
        StorageTier::Persistent,
        &DailyExecutionCapKey::Counter(user.clone()),
        counter,
    );
}

pub fn get_daily_cap_config(env: &Env, user: &Address) -> DailyCapConfig {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &DailyExecutionCapKey::Config(user.clone()),
        DailyCapConfig::default(),
    )
}

pub fn set_daily_cap_config(env: &Env, user: &Address, config: &DailyCapConfig) {
    crud_set(
        env,
        StorageTier::Persistent,
        &DailyExecutionCapKey::Config(user.clone()),
        config,
    );
}
