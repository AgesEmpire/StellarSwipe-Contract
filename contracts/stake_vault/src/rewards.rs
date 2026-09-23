pub const XLM: i128 = 10_000_000;
pub const DEFAULT_AUTO_FUND_AMOUNT: i128 = 5_000 * XLM;

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolStatus {
    pub balance: i128,
    pub estimated_days_remaining: u32,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolLow {
    pub balance: i128,
    pub days_remaining: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPool {
    pub balance: i128,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
    pub treasury_balance: i128,
}

/// Emergency pause state for risk-bearing `stake_vault` operations.
///
/// While paused, risk-bearing entry points (deposits/stakes) are blocked, but
/// safe-exit paths (withdrawals) remain available so users can always exit.
/// Only the admin/owner may pause or unpause, and both operations are
/// idempotent: repeating a call is a no-op rather than an error.
#[derive(Clone, Debug, PartialEq)]
pub struct PauseState {
    pub paused: bool,
    pub paused_by: Option<u32>,
}

impl PauseState {
    pub fn new() -> Self {
        PauseState {
            paused: false,
            paused_by: None,
        }
    }
}

impl Default for PauseState {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of a pause/unpause request, surfaced to clients so they can
/// distinguish a real state change from an idempotent no-op.
#[derive(Clone, Debug, PartialEq)]
pub enum PauseOutcome {
    Paused,
    Unpaused,
    AlreadyPaused,
    AlreadyUnpaused,
}

impl PauseOutcome {
    pub fn is_paused(&self) -> bool {
        matches!(self, PauseOutcome::Paused | PauseOutcome::AlreadyPaused)
    }
}

/// Pause risk-bearing operations. Admin/owner-only; idempotent.
///
/// Returns `Err` if `caller` is not the authorized admin, otherwise returns the
/// resulting [`PauseOutcome`] (a repeated pause is `AlreadyPaused`, not an error).
pub fn pause(state: &mut PauseState, caller: u32, admin: u32) -> Result<PauseOutcome, &'static str> {
    if caller != admin {
        return Err("unauthorized: only admin may pause");
    }

    if state.paused {
        return Ok(PauseOutcome::AlreadyPaused);
    }

    state.paused = true;
    state.paused_by = Some(caller);
    Ok(PauseOutcome::Paused)
}

/// Unpause operations, restoring normal behavior. Admin/owner-only; idempotent.
///
/// Returns `Err` if `caller` is not the authorized admin, otherwise returns the
/// resulting [`PauseOutcome`] (a repeated unpause is `AlreadyUnpaused`, not an error).
pub fn unpause(state: &mut PauseState, caller: u32, admin: u32) -> Result<PauseOutcome, &'static str> {
    if caller != admin {
        return Err("unauthorized: only admin may unpause");
    }

    if !state.paused {
        return Ok(PauseOutcome::AlreadyUnpaused);
    }

    state.paused = false;
    state.paused_by = None;
    Ok(PauseOutcome::Unpaused)
}

/// Guard for risk-bearing entry points (deposit/stake).
///
/// Returns `Err` while paused so callers can reject the operation; safe-exit
/// paths such as withdrawals must NOT call this guard.
pub fn ensure_not_paused(state: &PauseState) -> Result<(), &'static str> {
    if state.paused {
        return Err("paused: risk-bearing operations are disabled");
    }
    Ok(())
}

pub fn get_rewards_pool_status(pool: &RewardsPool) -> RewardsPoolStatus {
    RewardsPoolStatus {
        balance: pool.balance,
        estimated_days_remaining: estimated_days_remaining(pool.balance, pool.daily_outflow),
        daily_outflow: pool.daily_outflow,
        auto_fund_threshold: pool.auto_fund_threshold,
    }
}

pub fn monitor_rewards_pool(pool: &mut RewardsPool) -> Option<RewardsPoolLow> {
    if pool.balance >= pool.auto_fund_threshold {
        return None;
    }

    let days_remaining = estimated_days_remaining(pool.balance, pool.daily_outflow);
    let fund_amount = DEFAULT_AUTO_FUND_AMOUNT.min(pool.treasury_balance);
    pool.balance += fund_amount;
    pool.treasury_balance -= fund_amount;

    Some(RewardsPoolLow {
        balance: pool.balance,
        days_remaining,
    })
}

fn estimated_days_remaining(balance: i128, daily_outflow: i128) -> u32 {
    if daily_outflow <= 0 {
        return u32::MAX;
    }

    (balance.max(0) / daily_outflow) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_pool_status() {
        let mut pool = RewardsPool {
            balance: 10_000 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
        };

        assert_eq!(
            get_rewards_pool_status(&pool),
            RewardsPoolStatus {
                balance: 10_000 * XLM,
                estimated_days_remaining: 100,
                daily_outflow: 100 * XLM,
                auto_fund_threshold: 1_000 * XLM,
            }
        );
        assert_eq!(monitor_rewards_pool(&mut pool), None);
    }

    #[test]
    fn low_pool_auto_funds_from_treasury() {
        let mut pool = RewardsPool {
            balance: 500 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
        };

        let event = monitor_rewards_pool(&mut pool).unwrap();

        assert_eq!(event.days_remaining, 5);
        assert_eq!(pool.balance, 5_500 * XLM);
        assert_eq!(pool.treasury_balance, 15_000 * XLM);
    }

    #[test]
    fn empty_pool_reports_zero_days_and_funds_available_treasury() {
        let mut pool = RewardsPool {
            balance: 0,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 800 * XLM,
        };

        let event = monitor_rewards_pool(&mut pool).unwrap();

        assert_eq!(event.days_remaining, 0);
        assert_eq!(pool.balance, 800 * XLM);
        assert_eq!(pool.treasury_balance, 0);
    }

    #[test]
    fn pause_blocks_risk_bearing_operations() {
        let mut state = PauseState::new();
        assert_eq!(ensure_not_paused(&state), Ok(()));

        assert_eq!(pause(&mut state, 7, 7), Ok(PauseOutcome::Paused));
        assert!(state.paused);
        assert_eq!(state.paused_by, Some(7));
        assert!(ensure_not_paused(&state).is_err());
    }

    #[test]
    fn pause_and_unpause_are_idempotent() {
        let mut state = PauseState::new();

        assert_eq!(pause(&mut state, 7, 7), Ok(PauseOutcome::Paused));
        assert_eq!(pause(&mut state, 7, 7), Ok(PauseOutcome::AlreadyPaused));
        assert!(state.paused);

        assert_eq!(unpause(&mut state, 7, 7), Ok(PauseOutcome::Unpaused));
        assert_eq!(unpause(&mut state, 7, 7), Ok(PauseOutcome::AlreadyUnpaused));
        assert!(!state.paused);
        assert_eq!(state.paused_by, None);
        assert_eq!(ensure_not_paused(&state), Ok(()));
    }

    #[test]
    fn only_admin_may_pause_or_unpause() {
        let mut state = PauseState::new();

        assert!(pause(&mut state, 1, 7).is_err());
        assert!(!state.paused);

        assert_eq!(pause(&mut state, 7, 7), Ok(PauseOutcome::Paused));
        assert!(unpause(&mut state, 1, 7).is_err());
        assert!(state.paused);
    }

    #[test]
    fn safe_exit_paths_remain_available_during_pause() {
        let mut state = PauseState::new();
        assert_eq!(pause(&mut state, 7, 7), Ok(PauseOutcome::Paused));

        // Risk-bearing entry points are blocked...
        assert!(ensure_not_paused(&state).is_err());
        // ...but the pause state itself does not gate withdrawals, which must
        // remain callable so users can always exit.
        assert!(state.paused);
    }
}
