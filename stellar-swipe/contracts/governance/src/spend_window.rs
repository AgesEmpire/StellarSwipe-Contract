//! Rolling budget windows for governance treasury spend (Issue #1130).
//!
//! Caps the *aggregate* amount spent inside a fixed-length window so several
//! individually valid proposals executing close together cannot drain the
//! treasury. Windows are aligned to `window_secs` boundaries from the anchor
//! recorded at configuration time, so rollover is deterministic: the window
//! containing `now` is always `anchor + k * window_secs` for the unique `k`.

use soroban_sdk::{contracttype, symbol_short, Env, Symbol};

use crate::errors::GovernanceError;

const CONFIG_KEY: Symbol = symbol_short!("spwcfg");
const STATE_KEY: Symbol = symbol_short!("spwst");

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpendWindowConfig {
    /// Window length in seconds (> 0).
    pub window_secs: u64,
    /// Maximum aggregate spend allowed inside one window (> 0).
    pub cap: i128,
    /// Timestamp windows are aligned to.
    pub anchor: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpendWindowState {
    pub window_start: u64,
    pub spent: i128,
}

pub fn configure(env: &Env, window_secs: u64, cap: i128) -> Result<(), GovernanceError> {
    if window_secs == 0 {
        return Err(GovernanceError::InvalidDuration);
    }
    if cap <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let now = env.ledger().timestamp();
    env.storage().instance().set(
        &CONFIG_KEY,
        &SpendWindowConfig {
            window_secs,
            cap,
            anchor: now,
        },
    );
    env.storage().instance().set(
        &STATE_KEY,
        &SpendWindowState {
            window_start: now,
            spent: 0,
        },
    );
    #[allow(deprecated)]
    env.events()
        .publish((symbol_short!("spwin"), symbol_short!("config")), (window_secs, cap, now));
    Ok(())
}

pub fn get_config(env: &Env) -> Option<SpendWindowConfig> {
    env.storage().instance().get(&CONFIG_KEY)
}

fn window_start_for(cfg: &SpendWindowConfig, now: u64) -> u64 {
    let elapsed = now.saturating_sub(cfg.anchor);
    cfg.anchor + (elapsed - elapsed % cfg.window_secs)
}

/// Current window state after deterministic rollover (read-only).
pub fn current(env: &Env) -> Option<(SpendWindowConfig, SpendWindowState)> {
    let cfg = get_config(env)?;
    let start = window_start_for(&cfg, env.ledger().timestamp());
    let state: SpendWindowState = env
        .storage()
        .instance()
        .get(&STATE_KEY)
        .unwrap_or(SpendWindowState {
            window_start: start,
            spent: 0,
        });
    let state = if state.window_start == start {
        state
    } else {
        SpendWindowState {
            window_start: start,
            spent: 0,
        }
    };
    Some((cfg, state))
}

/// Remaining capacity in the current window; `i128::MAX` when unconfigured.
pub fn remaining(env: &Env) -> i128 {
    match current(env) {
        Some((cfg, st)) => cfg.cap.saturating_sub(st.spent).max(0),
        None => i128::MAX,
    }
}

/// Consume `amount` from the current window. Must be called before any
/// transfer/booking so an exhausted window fails the spend with no effects.
/// No-op when no window is configured.
pub fn consume(env: &Env, amount: i128) -> Result<(), GovernanceError> {
    let Some((cfg, mut st)) = current(env) else {
        return Ok(());
    };
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let new_spent = st
        .spent
        .checked_add(amount)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if new_spent > cfg.cap {
        return Err(GovernanceError::BudgetExceeded);
    }
    st.spent = new_spent;
    env.storage().instance().set(&STATE_KEY, &st);
    let remaining = cfg
        .cap
        .checked_sub(new_spent)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("spwin"), symbol_short!("consume")),
        (st.window_start, amount, new_spent, remaining),
    );
    Ok(())
}
