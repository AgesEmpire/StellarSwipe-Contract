#![allow(dead_code)]
use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::AutoTradeError;

const PRECISION: i128 = 1_000_000;

// ── Types ────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DCAFrequency {
    Daily,
    Weekly,
    Biweekly,
    Monthly,
    Custom(u64),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DCAStatus {
    Active,
    Paused,
    Completed,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DCAPurchase {
    pub timestamp: u64,
    pub amount_invested: i128,
    pub asset_acquired: i128,
    pub price: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DCAStrategy {
    pub user: Address,
    pub asset_pair: u32, // asset_id used as pair identifier (matches existing codebase u32 asset)
    pub purchase_amount: i128,
    pub frequency: DCAFrequency,
    pub start_time: u64,
    pub end_time: u64, // 0 = no end
    pub last_purchase: u64,
    pub total_invested: i128,
    pub total_amount_acquired: i128,
    pub average_entry_price: i128,
    pub purchases: Vec<DCAPurchase>,
    pub status: DCAStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DCAPerformance {
    pub total_invested: i128,
    pub current_value: i128,
    pub unrealized_pnl: i128,
    pub unrealized_pnl_pct: i32,
    pub average_entry_price: i128,
    pub current_price: i128,
    pub price_diff_pct: i32,
    pub total_purchases: u32,
    pub consistency_pct: u32,
}

#[contracttype]
pub enum DCAKey {
    Strategy(u64),
    NextId,
    ActiveIds,
}

// ── Storage helpers ──────────────────────────────────────────────────────────

fn next_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .persistent()
        .get(&DCAKey::NextId)
        .unwrap_or(0);
    env.storage().persistent().set(&DCAKey::NextId, &(id + 1));
    id
}

fn load(env: &Env, id: u64) -> Result<DCAStrategy, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&DCAKey::Strategy(id))
        .ok_or(AutoTradeError::DcaStrategyNotFound)
}

fn save(env: &Env, id: u64, s: &DCAStrategy) {
    env.storage().persistent().set(&DCAKey::Strategy(id), s);
}

fn active_ids(env: &Env) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&DCAKey::ActiveIds)
        .unwrap_or_else(|| Vec::new(env))
}

fn push_active_id(env: &Env, id: u64) {
    let mut ids = active_ids(env);
    ids.push_back(id);
    env.storage().persistent().set(&DCAKey::ActiveIds, &ids);
}

fn remove_active_id(env: &Env, target: u64) {
    let ids = active_ids(env);
    let mut updated: Vec<u64> = Vec::new(env);
    for i in 0..ids.len() {
        let v = ids.get(i).unwrap();
        if v != target {
            updated.push_back(v);
        }
    }
    env.storage().persistent().set(&DCAKey::ActiveIds, &updated);
}

// ── Interval helper ──────────────────────────────────────────────────────────

fn interval_secs(freq: &DCAFrequency) -> u64 {
    match freq {
        DCAFrequency::Daily => 86_400,
        DCAFrequency::Weekly => 604_800,
        DCAFrequency::Biweekly => 1_209_600,
        DCAFrequency::Monthly => 2_592_000,
        DCAFrequency::Custom(interval_seconds) => *interval_seconds,
    }
}

// ── Simulated trade execution (mirrors sdex.rs pattern) ─────────────────────

fn sim_execute_buy(env: &Env, asset_id: u32, amount: i128) -> Result<(i128, i128), AutoTradeError> {
    let price_key = (symbol_short!("price"), asset_id);
    let price: i128 = env
        .storage()
        .temporary()
        .get(&price_key)
        .unwrap_or(100_i128);

    let acquired = (amount * PRECISION) / price;
    Ok((acquired, price))
}

fn get_balance(env: &Env, user: &Address) -> i128 {
    env.storage()
        .temporary()
        .get(&(user.clone(), symbol_short!("balance")))
        .unwrap_or(0)
}

// ── Core functions ───────────────────────────────────────────────────────────

pub fn create_dca_strategy(
    env: &Env,
    user: Address,
    asset_pair: u32,
    purchase_amount: i128,
    frequency: DCAFrequency,
    duration_days: Option<u64>,
) -> Result<u64, AutoTradeError> {
    if purchase_amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    let now = env.ledger().timestamp();
    let end_time = duration_days.map(|d| now + d * 86_400).unwrap_or(0);
    let id = next_id(env);

    let strategy = DCAStrategy {
        user: user.clone(),
        asset_pair,
        purchase_amount,
        frequency,
        start_time: now,
        end_time,
        last_purchase: 0,
        total_invested: 0,
        total_amount_acquired: 0,
        average_entry_price: 0,
        purchases: Vec::new(env),
        status: DCAStatus::Active,
    };

    save(env, id, &strategy);
    push_active_id(env, id);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "dca_created"), user, id),
        purchase_amount,
    );

    Ok(id)
}

pub fn is_purchase_due(env: &Env, id: u64) -> Result<bool, AutoTradeError> {
    let s = load(env, id)?;

    if s.status != DCAStatus::Active {
        return Ok(false);
    }

    let now = env.ledger().timestamp();

    if s.end_time != 0 && now >= s.end_time {
        return Ok(false);
    }

    let next = if s.last_purchase == 0 {
        s.start_time
    } else {
        s.last_purchase + interval_secs(&s.frequency)
    };

    Ok(now >= next)
}

pub fn execute_dca_purchase(env: &Env, id: u64) -> Result<(), AutoTradeError> {
    let mut s = load(env, id)?;

    if s.status != DCAStatus::Active {
        return Err(AutoTradeError::DcaStrategyInactive);
    }

    let now = env.ledger().timestamp();

    if s.end_time != 0 && now >= s.end_time {
        s.status = DCAStatus::Completed;
        save(env, id, &s);
        remove_active_id(env, id);
        return Err(AutoTradeError::DcaEndTimeReached);
    }

    let balance = get_balance(env, &s.user);
    if balance < s.purchase_amount {
        s.status = DCAStatus::Paused;
        save(env, id, &s);
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "dca_paused_funds"), s.user.clone(), id),
            balance,
        );
        return Err(AutoTradeError::InsufficientBalance);
    }

    let (acquired, price) = sim_execute_buy(env, s.asset_pair, s.purchase_amount)?;

    s.total_invested += s.purchase_amount;
    s.total_amount_acquired += acquired;
    s.average_entry_price = (s.total_invested * PRECISION) / s.total_amount_acquired;
    s.last_purchase = now;

    s.purchases.push_back(DCAPurchase {
        timestamp: now,
        amount_invested: s.purchase_amount,
        asset_acquired: acquired,
        price,
    });

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "dca_purchase"), s.user.clone(), id),
        (s.purchase_amount, acquired, price, s.average_entry_price),
    );

    save(env, id, &s);
    Ok(())
}

pub fn execute_due_dca_purchases(env: &Env) -> Vec<u64> {
    let ids = active_ids(env);
    let mut executed: Vec<u64> = Vec::new(env);

    for i in 0..ids.len() {
        let id = ids.get(i).unwrap();
        if is_purchase_due(env, id).unwrap_or(false) {
            match execute_dca_purchase(env, id) {
                Ok(_) => executed.push_back(id),
                Err(e) => {
                    #[allow(deprecated)]
                    env.events().publish(
                        (Symbol::new(env, "dca_failed"), id),
                        e as u32,
                    );
                }
            }
        }
    }

    executed
}

pub fn handle_missed_dca_purchases(env: &Env, id: u64) -> Result<u32, AutoTradeError> {
    let s = load(env, id)?;
    let expected = calculate_expected_purchases(env, &s);
    let actual = s.purchases.len();

    if expected > actual {
        let missed = expected - actual;
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "dca_missed"), id),
            missed,
        );
        for _ in 0..missed {
            execute_dca_purchase(env, id)?;
        }
        return Ok(missed);
    }

    Ok(0)
}

pub fn update_dca_schedule(
    env: &Env,
    id: u64,
    new_amount: Option<i128>,
    new_frequency: Option<DCAFrequency>,
) -> Result<(), AutoTradeError> {
    let mut s = load(env, id)?;

    if let Some(amount) = new_amount {
        if amount <= 0 {
            return Err(AutoTradeError::InvalidAmount);
        }
        s.purchase_amount = amount;
    }

    if let Some(freq) = new_frequency {
        s.frequency = freq;
    }

    save(env, id, &s);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "dca_updated"), id),
        s.purchase_amount,
    );

    Ok(())
}

pub fn pause_dca_strategy(env: &Env, id: u64) -> Result<(), AutoTradeError> {
    let mut s = load(env, id)?;
    s.status = DCAStatus::Paused;
    save(env, id, &s);

    #[allow(deprecated)]
    env.events()
        .publish((Symbol::new(env, "dca_paused"), id), ());

    Ok(())
}

pub fn resume_dca_strategy(env: &Env, id: u64) -> Result<(), AutoTradeError> {
    let mut s = load(env, id)?;
    s.status = DCAStatus::Active;
    save(env, id, &s);

    #[allow(deprecated)]
    env.events()
        .publish((Symbol::new(env, "dca_resumed"), id), ());

    Ok(())
}

pub fn analyze_dca_performance(env: &Env, id: u64) -> Result<DCAPerformance, AutoTradeError> {
    let s = load(env, id)?;

    let price_key = (symbol_short!("price"), s.asset_pair);
    let current_price: i128 = env
        .storage()
        .temporary()
        .get(&price_key)
        .unwrap_or(100_i128);

    let current_value = (s.total_amount_acquired * current_price) / PRECISION;
    let unrealized_pnl = current_value - s.total_invested;

    let unrealized_pnl_pct = if s.total_invested > 0 {
        ((unrealized_pnl * 10_000) / s.total_invested) as i32
    } else {
        0
    };

    let price_diff_pct = if s.average_entry_price > 0 {
        (((current_price - s.average_entry_price) * 10_000) / s.average_entry_price) as i32
    } else {
        0
    };

    let total_purchases = s.purchases.len();
    let expected = calculate_expected_purchases(env, &s);
    let consistency_pct = if expected > 0 {
        (total_purchases * 10_000) / expected
    } else {
        10_000
    };

    Ok(DCAPerformance {
        total_invested: s.total_invested,
        current_value,
        unrealized_pnl,
        unrealized_pnl_pct,
        average_entry_price: s.average_entry_price,
        current_price,
        price_diff_pct,
        total_purchases,
        consistency_pct,
    })
}

pub fn get_dca_strategy(env: &Env, id: u64) -> Result<DCAStrategy, AutoTradeError> {
    load(env, id)
}

fn calculate_expected_purchases(_env: &Env, s: &DCAStrategy) -> u32 {
    let now = _env.ledger().timestamp();
    let elapsed = now.saturating_sub(s.start_time);
    let interval = interval_secs(&s.frequency);
    if interval == 0 {
        return 0;
    }
    (elapsed / interval) as u32
}
