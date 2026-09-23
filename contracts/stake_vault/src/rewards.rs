pub const XLM: i128 = 10_000_000;
pub const DEFAULT_AUTO_FUND_AMOUNT: i128 = 5_000 * XLM;

/// Decimal precision (in base-10 digits) assumed by every amount conversion in
/// this module. All on-chain amounts are stored as integer base units scaled by
/// `10^EXPECTED_DECIMALS`, so a token whose contract reports a different
/// precision cannot be mixed with these balances without silent value loss.
pub const EXPECTED_DECIMALS: u32 = 7;

/// Scale factor implied by [`EXPECTED_DECIMALS`] (`10^EXPECTED_DECIMALS`).
pub const DECIMAL_SCALE: i128 = XLM;

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

/// Tracks the last applied deposit so repeated attempts for the same
/// transaction cannot duplicate balances or rewards.
#[derive(Clone, Debug, PartialEq)]
pub struct DepositRetryGuard {
    pub last_deposit_id: u64,
    pub last_deposit_amount: i128,
    pub applied: bool,
}

impl DepositRetryGuard {
    pub fn new() -> Self {
        DepositRetryGuard {
            last_deposit_id: 0,
            last_deposit_amount: 0,
            applied: false,
        }
    }
}

/// Metadata a token contract is expected to expose at `stake_vault` setup.
///
/// `address` is the token contract identifier, `decimals` its reported
/// precision, and `is_contract` whether the address actually resolves to a
/// deployed contract (as opposed to an account or an empty/invalid address).
#[derive(Clone, Debug, PartialEq)]
pub struct TokenMetadata {
    pub address: u32,
    pub decimals: u32,
    pub is_contract: bool,
}

/// Immutable token configuration captured at `stake_vault` initialization.
///
/// Once validated, the token address and its precision are frozen: there is no
/// setter, so changing them requires a governed upgrade/redeploy rather than a
/// runtime mutation.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenConfig {
    pub address: u32,
    pub decimals: u32,
}

/// Validate a token contract before `stake_vault` initialization.
///
/// Rejects invalid/zero addresses, addresses that are not contracts, and
/// contracts whose reported decimals differ from [`EXPECTED_DECIMALS`]. On
/// success returns the frozen [`TokenConfig`] used for all amount conversions.
pub fn validate_token_metadata(metadata: &TokenMetadata) -> Result<TokenConfig, &'static str> {
    if metadata.address == 0 {
        return Err("invalid token: address must be non-zero");
    }

    if !metadata.is_contract {
        return Err("invalid token: address is not a contract");
    }

    if metadata.decimals != EXPECTED_DECIMALS {
        return Err("incompatible token: decimals do not match vault precision");
    }

    Ok(TokenConfig {
        address: metadata.address,
        decimals: metadata.decimals,
    })
}

/// Convert a whole-token amount into base units using checked arithmetic.
///
/// Rounding: this conversion is exact for whole tokens; any fractional part is
/// truncated toward zero (integer division), never rounded up. Overflow and
/// negative inputs are rejected rather than saturating.
pub fn to_base_units(amount: i128, config: &TokenConfig) -> Result<i128, &'static str> {
    if amount < 0 {
        return Err("invalid amount: must be non-negative");
    }

    if config.decimals != EXPECTED_DECIMALS {
        return Err("incompatible token: decimals do not match vault precision");
    }

    amount
        .checked_mul(DECIMAL_SCALE)
        .ok_or("amount overflow: base unit conversion failed")
}

/// Convert base units back into whole tokens using checked arithmetic.
///
/// Rounding: the fractional remainder is truncated toward zero, so the result
/// is always `<=` the exact value (never rounded up).
pub fn from_base_units(base_units: i128, config: &TokenConfig) -> Result<i128, &'static str> {
    if base_units < 0 {
        return Err("invalid amount: must be non-negative");
    }

    if config.decimals != EXPECTED_DECIMALS {
        return Err("incompatible token: decimals do not match vault precision");
    }

    Ok(base_units / DECIMAL_SCALE)
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
        }
    }
}

impl Default for DepositRetryGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for PauseState {
    fn default() -> Self {
        Self::new()
    }
}
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of attempting to apply a deposit against the retry guard.
#[derive(Clone, Debug, PartialEq)]
pub enum DepositOutcome {
    /// Deposit was applied for the first time.
    Applied { deposit_id: u64, amount: i128 },
    /// Deposit was already applied; no state change was made.
    AlreadyApplied { deposit_id: u64 },
    /// Deposit was rejected (invalid amount); state left untouched.
    Rejected { deposit_id: u64 },
}

/// Applies a deposit to the rewards pool exactly once per `deposit_id`.
///
/// Repeated attempts with the same `deposit_id` are detected via the
/// `DepositRetryGuard` marker and skipped, so balances and rewards are never
/// duplicated. Invalid amounts are rejected without mutating state, leaving
/// the contract consistent for a later retry.
pub fn apply_deposit(
    pool: &mut RewardsPool,
    guard: &mut DepositRetryGuard,
    deposit_id: u64,
    amount: i128,
) -> DepositOutcome {
    if amount <= 0 {
        return DepositOutcome::Rejected { deposit_id };
    }

    if guard.applied && guard.last_deposit_id == deposit_id {
        return DepositOutcome::AlreadyApplied { deposit_id };
    }

    pool.balance += amount;
    guard.last_deposit_id = deposit_id;
    guard.last_deposit_amount = amount;
    guard.applied = true;

    DepositOutcome::Applied { deposit_id, amount }
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

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderCapConfig {
    pub max_provider_stake: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderCapExceeded {
    pub attempted: i128,
    pub cap: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StakeAllocated {
    pub amount: i128,
    pub total_stake: i128,
}

pub fn set_provider_cap(max_provider_stake: i128) -> Result<ProviderCapConfig, &'static str> {
    if max_provider_stake <= 0 {
        return Err("provider cap must be positive");
    }

    Ok(ProviderCapConfig {
        max_provider_stake,
    })
}

pub fn allocate_stake(
    config: &ProviderCapConfig,
    current_stake: i128,
    amount: i128,
) -> Result<StakeAllocated, ProviderCapExceeded> {
    let attempted = current_stake + amount;

    if attempted > config.max_provider_stake {
        return Err(ProviderCapExceeded {
            attempted,
            cap: config.max_provider_stake,
        });
    }

    Ok(StakeAllocated {
        amount,
        total_stake: attempted,
    })
}
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

    fn sample_pool() -> RewardsPool {
        RewardsPool {
            balance: 10_000 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
        }
    }

    fn valid_metadata() -> TokenMetadata {
        TokenMetadata {
            address: 42,
            decimals: EXPECTED_DECIMALS,
            is_contract: true,
        }
    }
        }
    }

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
    fn deposit_applies_once() {
        let mut pool = sample_pool();
        let mut guard = DepositRetryGuard::new();

        let outcome = apply_deposit(&mut pool, &mut guard, 1, 250 * XLM);

        assert_eq!(
            outcome,
            DepositOutcome::Applied {
                deposit_id: 1,
                amount: 250 * XLM,
            }
        );
        assert_eq!(pool.balance, 10_250 * XLM);
    }

    #[test]
    fn repeated_deposit_is_not_duplicated() {
        let mut pool = sample_pool();
        let mut guard = DepositRetryGuard::new();

        apply_deposit(&mut pool, &mut guard, 7, 100 * XLM);
        let retry = apply_deposit(&mut pool, &mut guard, 7, 100 * XLM);

        assert_eq!(retry, DepositOutcome::AlreadyApplied { deposit_id: 7 });
        assert_eq!(pool.balance, 10_100 * XLM);
    }

    #[test]
    fn rejected_deposit_leaves_state_consistent() {
        let mut pool = sample_pool();
        let mut guard = DepositRetryGuard::new();

        let outcome = apply_deposit(&mut pool, &mut guard, 3, 0);

        assert_eq!(outcome, DepositOutcome::Rejected { deposit_id: 3 });
        assert_eq!(pool.balance, 10_000 * XLM);
        assert!(!guard.applied);

        // A valid retry after rejection still applies cleanly.
        let retry = apply_deposit(&mut pool, &mut guard, 3, 50 * XLM);
        assert_eq!(
            retry,
            DepositOutcome::Applied {
                deposit_id: 3,
                amount: 50 * XLM,
            }
        );
        assert_eq!(pool.balance, 10_050 * XLM);
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

    #[test]
    fn valid_token_metadata_is_accepted_and_frozen() {
        let config = validate_token_metadata(&valid_metadata()).unwrap();
        assert_eq!(config.address, 42);
        assert_eq!(config.decimals, EXPECTED_DECIMALS);
    }

    #[test]
    fn zero_address_is_rejected() {
        let metadata = TokenMetadata {
            address: 0,
            ..valid_metadata()
        };
        assert!(validate_token_metadata(&metadata).is_err());
    }

    #[test]
    fn non_contract_address_is_rejected() {
        let metadata = TokenMetadata {
            is_contract: false,
            ..valid_metadata()
        };
        assert!(validate_token_metadata(&metadata).is_err());
    }

    #[test]
    fn mismatched_decimals_are_rejected() {
        let metadata = TokenMetadata {
            decimals: EXPECTED_DECIMALS + 1,
            ..valid_metadata()
        };
        assert!(validate_token_metadata(&metadata).is_err());

        let metadata = TokenMetadata {
            decimals: 0,
            ..valid_metadata()
        };
        assert!(validate_token_metadata(&metadata).is_err());
    }

    #[test]
    fn malicious_metadata_cannot_bypass_validation() {
        // A hostile contract may report a huge precision or claim to be a
        // contract while pointing at a zero address; both must be rejected.
        let metadata = TokenMetadata {
            address: 0,
            decimals: u32::MAX,
            is_contract: true,
        };
        assert!(validate_token_metadata(&metadata).is_err());

        let metadata = TokenMetadata {
            address: 99,
            decimals: u32::MAX,
            is_contract: true,
        };
        assert!(validate_token_metadata(&metadata).is_err());
    }

    #[test]
    fn conversions_use_checked_arithmetic_and_truncate() {
        let config = validate_token_metadata(&valid_metadata()).unwrap();

        assert_eq!(to_base_units(3, &config), Ok(3 * XLM));
        assert_eq!(from_base_units(3 * XLM + 1, &config), Ok(3));

        // Overflow is rejected rather than wrapping/saturating.
        assert!(to_base_units(i128::MAX, &config).is_err());
        // Negative amounts are rejected.
        assert!(to_base_units(-1, &config).is_err());
        assert!(from_base_units(-1, &config).is_err());
    }

    #[test]
    fn conversions_reject_incompatible_config() {
        let config = TokenConfig {
            address: 42,
            decimals: EXPECTED_DECIMALS + 2,
        };
        assert!(to_base_units(1, &config).is_err());
        assert!(from_base_units(XLM, &config).is_err());
    }

    #[test]
    fn provider_cap_rejects_non_positive_values() {
        assert_eq!(set_provider_cap(0), Err("provider cap must be positive"));
        assert_eq!(set_provider_cap(-1), Err("provider cap must be positive"));
        assert_eq!(
            set_provider_cap(1_000 * XLM),
            Ok(ProviderCapConfig {
                max_provider_stake: 1_000 * XLM,
            })
        );
    }

    #[test]
    fn allocation_over_cap_is_rejected_before_mutation() {
        let config = set_provider_cap(1_000 * XLM).unwrap();
        let current_stake = 900 * XLM;

        let result = allocate_stake(&config, current_stake, 200 * XLM);

        assert_eq!(
            result,
            Err(ProviderCapExceeded {
                attempted: 1_100 * XLM,
                cap: 1_000 * XLM,
            })
        );
        assert_eq!(current_stake, 900 * XLM);
    }

    #[test]
    fn allocation_within_cap_records_accepted_values() {
        let config = set_provider_cap(1_000 * XLM).unwrap();

        let result = allocate_stake(&config, 900 * XLM, 100 * XLM).unwrap();

        assert_eq!(
            result,
            StakeAllocated {
                amount: 100 * XLM,
                total_stake: 1_000 * XLM,
            }
        );
    }
    }
}
