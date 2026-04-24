#![allow(dead_code)]
use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::AutoTradeError;

// ─── Types ───────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeRateLimits {
    pub per_user_hourly_transfers: u32,
    pub per_user_hourly_volume: i128,
    pub per_user_daily_transfers: u32,
    pub per_user_daily_volume: i128,
    pub global_hourly_capacity: u32,
    pub global_daily_volume: i128,
    pub min_transfer_amount: i128,
    pub cooldown_between_transfers: u64,
}

impl Default for BridgeRateLimits {
    fn default() -> Self {
        BridgeRateLimits {
            per_user_hourly_transfers: 10,
            per_user_hourly_volume: 1_000_000_0000000,
            per_user_daily_transfers: 50,
            per_user_daily_volume: 5_000_000_0000000,
            global_hourly_capacity: 1000,
            global_daily_volume: 100_000_000_0000000,
            min_transfer_amount: 1_0000000,
            cooldown_between_transfers: 30,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRecord {
    pub timestamp: u64,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserTransferHistory {
    pub transfers_last_hour: Vec<TransferRecord>,
    pub transfers_last_day: Vec<TransferRecord>,
    pub last_transfer_time: u64,
    pub violation_count: u32,
    pub penalty_until: u64, // 0 = no penalty
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViolationType {
    HourlyCountExceeded,
    HourlyVolumeExceeded,
    DailyCountExceeded,
    DailyVolumeExceeded,
    GlobalCapacityExceeded,
    CooldownViolation,
    BelowMinimum,
}

// ─── Storage Keys ─────────────────────────────────────────────────────────────

#[contracttype]
pub enum RateLimitKey {
    Limits,
    UserHistory(Address),
    Whitelist,
    GlobalHourlyCount,
    Admin,
}

// ─── Storage Helpers ──────────────────────────────────────────────────────────

pub fn get_limits(env: &Env) -> BridgeRateLimits {
    env.storage()
        .persistent()
        .get(&RateLimitKey::Limits)
        .unwrap_or_default()
}

pub fn set_limits(env: &Env, limits: &BridgeRateLimits) {
    env.storage()
        .persistent()
        .set(&RateLimitKey::Limits, limits);
}

fn get_history(env: &Env, user: &Address) -> UserTransferHistory {
    env.storage()
        .persistent()
        .get(&RateLimitKey::UserHistory(user.clone()))
        .unwrap_or(UserTransferHistory {
            transfers_last_hour: Vec::new(env),
            transfers_last_day: Vec::new(env),
            last_transfer_time: 0,
            violation_count: 0,
            penalty_until: 0,
        })
}

fn set_history(env: &Env, user: &Address, history: &UserTransferHistory) {
    env.storage()
        .persistent()
        .set(&RateLimitKey::UserHistory(user.clone()), history);
}

fn get_whitelist(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&RateLimitKey::Whitelist)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_whitelist(env: &Env, list: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&RateLimitKey::Whitelist, list);
}

fn get_global_hourly_count(env: &Env) -> u32 {
    env.storage()
        .temporary()
        .get(&RateLimitKey::GlobalHourlyCount)
        .unwrap_or(0)
}

fn set_global_hourly_count(env: &Env, count: u32) {
    env.storage()
        .temporary()
        .set(&RateLimitKey::GlobalHourlyCount, &count);
}

pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&RateLimitKey::Admin)
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage()
        .persistent()
        .set(&RateLimitKey::Admin, admin);
}

fn require_admin(env: &Env) -> Result<(), AutoTradeError> {
    let admin = get_admin(env).ok_or(AutoTradeError::Unauthorized)?;
    admin.require_auth();
    Ok(())
}

// ─── Prune ────────────────────────────────────────────────────────────────────

fn prune_old_records(env: &Env, history: &mut UserTransferHistory, now: u64) {
    let hour_ago = now.saturating_sub(3600);
    let day_ago = now.saturating_sub(86400);

    let mut new_hour: Vec<TransferRecord> = Vec::new(env);
    for i in 0..history.transfers_last_hour.len() {
        if let Some(r) = history.transfers_last_hour.get(i) {
            if r.timestamp > hour_ago {
                new_hour.push_back(r);
            }
        }
    }

    let mut new_day: Vec<TransferRecord> = Vec::new(env);
    for i in 0..history.transfers_last_day.len() {
        if let Some(r) = history.transfers_last_day.get(i) {
            if r.timestamp > day_ago {
                new_day.push_back(r);
            }
        }
    }

    history.transfers_last_hour = new_hour;
    history.transfers_last_day = new_day;
}

// ─── Whitelist ────────────────────────────────────────────────────────────────

pub fn is_whitelisted(env: &Env, user: &Address) -> bool {
    let list = get_whitelist(env);
    for i in 0..list.len() {
        if let Some(addr) = list.get(i) {
            if addr == *user {
                return true;
            }
        }
    }
    false
}

pub fn add_to_whitelist(env: &Env, user: &Address) -> Result<(), AutoTradeError> {
    require_admin(env)?;
    if !is_whitelisted(env, user) {
        let mut list = get_whitelist(env);
        list.push_back(user.clone());
        set_whitelist(env, &list);
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "user_whitelisted"), user.clone()),
            (),
        );
    }
    Ok(())
}

pub fn remove_from_whitelist(env: &Env, user: &Address) -> Result<(), AutoTradeError> {
    require_admin(env)?;
    let list = get_whitelist(env);
    let mut new_list: Vec<Address> = Vec::new(env);
    for i in 0..list.len() {
        if let Some(addr) = list.get(i) {
            if addr != *user {
                new_list.push_back(addr);
            }
        }
    }
    set_whitelist(env, &new_list);
    Ok(())
}

// ─── Penalty ──────────────────────────────────────────────────────────────────

pub fn record_violation(
    env: &Env,
    user: &Address,
    violation_type: ViolationType,
) -> Result<(), AutoTradeError> {
    let mut history = get_history(env, user);
    history.violation_count += 1;

    let penalty_duration: u64 = match history.violation_count {
        1..=2 => 3600,
        3..=5 => 86400,
        6..=10 => 604800,
        _ => 2592000,
    };

    let now = env.ledger().timestamp();
    history.penalty_until = now + penalty_duration;
    set_history(env, user, &history);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "rate_limit_violation"),
            user.clone(),
        ),
        (violation_type, penalty_duration, history.violation_count),
    );

    Ok(())
}

// ─── Check ────────────────────────────────────────────────────────────────────

pub fn check_rate_limits(
    env: &Env,
    user: &Address,
    amount: i128,
) -> Result<(), AutoTradeError> {
    if is_whitelisted(env, user) {
        return Ok(());
    }

    let limits = get_limits(env);
    let now = env.ledger().timestamp();
    let mut history = get_history(env, user);

    // Penalty check
    if history.penalty_until > 0 && now < history.penalty_until {
        return Err(AutoTradeError::RateLimitPenalty);
    }

    // Minimum amount
    if amount < limits.min_transfer_amount {
        return Err(AutoTradeError::BelowMinTransfer);
    }

    // Cooldown
    if history.last_transfer_time > 0 {
        let elapsed = now.saturating_sub(history.last_transfer_time);
        if elapsed < limits.cooldown_between_transfers {
            return Err(AutoTradeError::CooldownNotElapsed);
        }
    }

    prune_old_records(env, &mut history, now);

    // Hourly count
    if history.transfers_last_hour.len() as u32 >= limits.per_user_hourly_transfers {
        return Err(AutoTradeError::HourlyTransferLimitExceeded);
    }

    // Hourly volume
    let mut hourly_vol: i128 = 0;
    for i in 0..history.transfers_last_hour.len() {
        if let Some(r) = history.transfers_last_hour.get(i) {
            hourly_vol += r.amount;
        }
    }
    if hourly_vol + amount > limits.per_user_hourly_volume {
        return Err(AutoTradeError::HourlyVolumeLimitExceeded);
    }

    // Daily count
    if history.transfers_last_day.len() as u32 >= limits.per_user_daily_transfers {
        return Err(AutoTradeError::DailyTransferLimitExceeded);
    }

    // Daily volume
    let mut daily_vol: i128 = 0;
    for i in 0..history.transfers_last_day.len() {
        if let Some(r) = history.transfers_last_day.get(i) {
            daily_vol += r.amount;
        }
    }
    if daily_vol + amount > limits.per_user_daily_volume {
        return Err(AutoTradeError::DailyVolumeLimitExceeded);
    }

    // Global hourly capacity
    let global_count = get_global_hourly_count(env);
    if global_count >= limits.global_hourly_capacity {
        return Err(AutoTradeError::GlobalCapacityExceeded);
    }

    Ok(())
}

// ─── Record ───────────────────────────────────────────────────────────────────

pub fn record_transfer(env: &Env, user: &Address, amount: i128) {
    let now = env.ledger().timestamp();
    let mut history = get_history(env, user);

    prune_old_records(env, &mut history, now);

    let record = TransferRecord { timestamp: now, amount };
    history.transfers_last_hour.push_back(record.clone());
    history.transfers_last_day.push_back(record);
    history.last_transfer_time = now;

    set_history(env, user, &history);
    set_global_hourly_count(env, get_global_hourly_count(env) + 1);
}

// ─── Dynamic Adjustment ───────────────────────────────────────────────────────

pub fn adjust_limits_based_on_load(env: &Env) -> Result<(), AutoTradeError> {
    let mut limits = get_limits(env);
    let current_load = get_global_hourly_count(env);
    let load_pct = if limits.global_hourly_capacity > 0 {
        (current_load as u64 * 100) / limits.global_hourly_capacity as u64
    } else {
        100
    };

    match load_pct {
        0..=50 => {
            limits.per_user_hourly_transfers =
                (limits.per_user_hourly_transfers + 1).min(20);
        }
        80..=100 => {
            limits.per_user_hourly_transfers =
                limits.per_user_hourly_transfers.saturating_sub(1).max(3);
            limits.cooldown_between_transfers =
                (limits.cooldown_between_transfers + 60).min(600);
        }
        _ => {}
    }

    set_limits(env, &limits);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("rl_adjust"),),
        (load_pct, limits.per_user_hourly_transfers),
    );

    Ok(())
}

// ─── Query helpers (for tests / contract exposure) ────────────────────────────

pub fn get_user_history(env: &Env, user: &Address) -> UserTransferHistory {
    get_history(env, user)
}
