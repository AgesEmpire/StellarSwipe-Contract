pub const INSURANCE_FEE_SHARE_BPS: i128 = 500;
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Maximum balance (in base units) that qualifies as sweepable dust.
///
/// Only fee_collector balances strictly below this threshold may be swept via
/// [`sweep_dust`]. Anything at or above this value is an ordinary fee balance
/// and must not be moved through the dust-sweep path.
pub const DUST_SWEEP_THRESHOLD: i128 = 1_000;

/// Schema version for fee_collector configuration events.
///
/// Bump this whenever the shape of [`ConfigChanged`] changes so indexers can
/// reconstruct configuration history across versions.
pub const CONFIG_EVENT_SCHEMA_VERSION: u32 = 1;

/// Documented topic emitted for every successful configuration mutation.
///
/// Matches `docs/event_schema.json` (`fee_collector.config_changed`).
pub const CONFIG_CHANGED_TOPIC: &str = "fee_collector.config_changed";

/// Documented topic emitted for every successful dust sweep.
///
/// Matches `docs/event_schema.json` (`fee_collector.dust_swept`).
pub const DUST_SWEPT_TOPIC: &str = "fee_collector.dust_swept";

/// Identifies which configuration field a [`ConfigChanged`] event refers to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConfigField {
    FeeRate,
    Recipient,
    Cap,
    Authorization,
}

/// Versioned event emitted exactly once per successful configuration change.
///
/// `old_value`/`new_value` are `None` when the value is not safe to disclose
/// (e.g. authorization settings); the affected field is always present.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigChanged<Actor> {
    pub schema_version: u32,
    pub topic: &'static str,
    pub field: ConfigField,
    pub old_value: Option<i128>,
    pub new_value: Option<i128>,
    pub actor: Actor,
}

/// Audit record emitted for every successful dust sweep.
///
/// Records the destination, the swept amount, and the authorizing caller so
/// the sweep can be reconstructed and audited off-chain.
#[derive(Clone, Debug, PartialEq)]
pub struct DustSwept<Actor, Destination> {
    pub schema_version: u32,
    pub topic: &'static str,
    pub destination: Destination,
    pub amount: i128,
    pub actor: Actor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InsurancePool {
    pub balance: i128,
    /// Protected treasury reserves that must never be withdrawn below the
    /// configured solvency floor.
    pub protected_reserves: i128,
    /// Minimum protected reserves that must remain after any withdrawal.
    pub solvency_floor: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeLoss {
    pub trade_id: u64,
    pub slashed_signal: bool,
    pub stop_loss_amount: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InsuranceClaimed<User> {
    pub user: User,
    pub trade_id: u64,
    pub amount_claimed: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContractError {
    InvalidClaim,
    LossWithinStopLoss,
    Unauthorized,
    /// Stable error returned when a withdrawal or payout would reduce the
    /// protected treasury reserves below their configured solvency floor.
    /// State is left unchanged when this is returned.
    SolvencyFloorBreached,
    InvalidSweep,
}

/// Emits the single documented event for a successful configuration change.
///
/// Callers must only invoke this after the mutation has been committed; a
/// rejected call returns `Err` before reaching this point and therefore emits
/// no misleading success event.
fn emit_config_changed<Actor: Clone>(
    field: ConfigField,
    old_value: Option<i128>,
    new_value: Option<i128>,
    actor: Actor,
) -> ConfigChanged<Actor> {
    ConfigChanged {
        schema_version: CONFIG_EVENT_SCHEMA_VERSION,
        topic: CONFIG_CHANGED_TOPIC,
        field,
        old_value,
        new_value,
        actor,
    }
}

/// Updates the insurance fee share and emits one versioned config event.
///
/// Rejects out-of-range rates (must be `0..=BPS_DENOMINATOR`) without emitting
/// a success event.
pub fn set_insurance_fee_share<Actor: Clone>(
    current_bps: &mut i128,
    new_bps: i128,
    actor: Actor,
) -> Result<ConfigChanged<Actor>, ContractError> {
    if new_bps < 0 || new_bps > BPS_DENOMINATOR {
        return Err(ContractError::Unauthorized);
    }

    let old_bps = *current_bps;
    *current_bps = new_bps;

    Ok(emit_config_changed(
        ConfigField::FeeRate,
        Some(old_bps),
        Some(new_bps),
        actor,
    ))
}

/// Updates the insurance pool cap and emits one versioned config event.
///
/// Rejects caps below the current balance without emitting a success event.
pub fn set_insurance_cap<Actor: Clone>(
    pool: &mut InsurancePool,
    new_cap: i128,
    actor: Actor,
) -> Result<ConfigChanged<Actor>, ContractError> {
    if new_cap < pool.balance {
        return Err(ContractError::Unauthorized);
    }

    let old_cap = pool.balance;
    pool.balance = new_cap;

    Ok(emit_config_changed(
        ConfigField::Cap,
        Some(old_cap),
        Some(new_cap),
        actor,
    ))
}

/// Updates authorization settings and emits one versioned config event.
///
/// Authorization values are not disclosed, so `old_value`/`new_value` are
/// `None` while the affected field and actor are still recorded.
pub fn set_authorization<Actor: Clone>(
    authorized: &mut bool,
    new_authorized: bool,
    actor: Actor,
) -> ConfigChanged<Actor> {
    *authorized = new_authorized;

    emit_config_changed(ConfigField::Authorization, None, None, actor)
}

/// Returns `true` when withdrawing `amount` from the pool would keep the
/// protected treasury reserves at or above the configured solvency floor.
///
/// This is a pure check so callers can validate the invariant before any
/// state mutation, guaranteeing atomicity.
pub fn solvency_invariant_holds(pool: &InsurancePool, amount: i128) -> bool {
    if amount < 0 {
        return false;
    }
    pool.protected_reserves.saturating_sub(amount) >= pool.solvency_floor
}

/// Sweeps a negligible fee_collector dust balance to an explicit destination.
///
/// Only balances strictly below [`DUST_SWEEP_THRESHOLD`] are sweepable, so
/// ordinary fee balances cannot be moved through this path. The caller must be
/// explicitly authorized, the destination must be non-zero, and the amount must
/// be positive and no greater than the current balance. On success the balance
/// is reduced and a [`DustSwept`] audit event is returned; rejected calls return
/// `Err` before any mutation and emit no event.
pub fn sweep_dust<Actor: Clone, Destination: Clone>(
    balance: &mut i128,
    destination: Destination,
    amount: i128,
    authorized: bool,
    actor: Actor,
) -> Result<DustSwept<Actor, Destination>, ContractError> {
    if !authorized {
        return Err(ContractError::Unauthorized);
    }
    if amount <= 0 || amount > *balance || amount >= DUST_SWEEP_THRESHOLD {
        return Err(ContractError::InvalidSweep);
    }
    if *balance >= DUST_SWEEP_THRESHOLD {
        return Err(ContractError::InvalidSweep);
    }

    *balance -= amount;

    Ok(DustSwept {
        schema_version: CONFIG_EVENT_SCHEMA_VERSION,
        topic: DUST_SWEPT_TOPIC,
        destination,
        amount,
        actor,
    })
}

pub fn allocate_insurance_fee(pool: &mut InsurancePool, collected_fee: i128) -> i128 {
    let insurance_share = collected_fee.saturating_mul(INSURANCE_FEE_SHARE_BPS) / BPS_DENOMINATOR;
    pool.balance = pool.balance.saturating_add(insurance_share);
    insurance_share
}

/// Withdraws `amount` from the protected treasury reserves.
///
/// The solvency invariant is enforced atomically: the check runs before any
/// mutation, so a violation returns [`ContractError::SolvencyFloorBreached`]
/// and leaves the pool completely unchanged.
pub fn withdraw_protected_reserves(
    pool: &mut InsurancePool,
    amount: i128,
) -> Result<i128, ContractError> {
    if !solvency_invariant_holds(pool, amount) {
        return Err(ContractError::SolvencyFloorBreached);
    }

    pool.protected_reserves -= amount;
    Ok(pool.protected_reserves)
}

pub fn claim_insurance<User: Clone>(
    pool: &mut InsurancePool,
    user: User,
    trade: &TradeLoss,
    loss_amount: i128,
) -> Result<InsuranceClaimed<User>, ContractError> {
    if !trade.slashed_signal {
        return Err(ContractError::InvalidClaim);
    }
    if loss_amount <= trade.stop_loss_amount {
        return Err(ContractError::LossWithinStopLoss);
    }

    let max_loss_payout = loss_amount / 2;
    let amount_claimed = max_loss_payout.min(pool.balance);

    // Enforce the treasury solvency invariant atomically before mutating any
    // state. A breach returns a stable error and leaves the pool untouched.
    if !solvency_invariant_holds(pool, amount_claimed) {
        return Err(ContractError::SolvencyFloorBreached);
    }

    pool.balance -= amount_claimed;
    pool.protected_reserves -= amount_claimed;

    Ok(InsuranceClaimed {
        user,
        trade_id: trade.trade_id,
        amount_claimed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(balance: i128, protected_reserves: i128, solvency_floor: i128) -> InsurancePool {
        InsurancePool {
            balance,
            protected_reserves,
            solvency_floor,
        }
    }

    #[test]
    fn valid_claim_pays_half_loss() {
        let mut pool = pool(1_000, 1_000, 0);
        let trade = TradeLoss {
            trade_id: 42,
            slashed_signal: true,
            stop_loss_amount: 100,
        };

        let event = claim_insurance(&mut pool, "user-1", &trade, 600).unwrap();

        assert_eq!(event.amount_claimed, 300);
        assert_eq!(event.trade_id, 42);
        assert_eq!(pool.balance, 700);
    }

    #[test]
    fn invalid_claim_without_slashed_signal_is_rejected() {
        let mut pool = pool(1_000, 1_000, 0);
        let trade = TradeLoss {
            trade_id: 43,
            slashed_signal: false,
            stop_loss_amount: 100,
        };

        let result = claim_insurance(&mut pool, "user-1", &trade, 600);

        assert_eq!(result, Err(ContractError::InvalidClaim));
        assert_eq!(pool.balance, 1_000);
    }

    #[test]
    fn pool_insufficient_caps_payout_at_balance() {
        let mut pool = pool(75, 75, 0);
        let trade = TradeLoss {
            trade_id: 44,
            slashed_signal: true,
            stop_loss_amount: 100,
        };

        let event = claim_insurance(&mut pool, "user-1", &trade, 600).unwrap();

        assert_eq!(event.amount_claimed, 75);
        assert_eq!(pool.balance, 0);
    }

    #[test]
    fn collected_fees_fund_pool_at_five_percent() {
        let mut pool = pool(0, 0, 0);

        let allocated = allocate_insurance_fee(&mut pool, 10_000);

        assert_eq!(allocated, 500);
        assert_eq!(pool.balance, 500);
    }

    #[test]
    fn fee_rate_change_emits_one_versioned_event() {
        let mut bps = INSURANCE_FEE_SHARE_BPS;

        let event = set_insurance_fee_share(&mut bps, 750, "admin").unwrap();

        assert_eq!(event.schema_version, CONFIG_EVENT_SCHEMA_VERSION);
        assert_eq!(event.topic, CONFIG_CHANGED_TOPIC);
        assert_eq!(event.field, ConfigField::FeeRate);
        assert_eq!(event.old_value, Some(500));
        assert_eq!(event.new_value, Some(750));
        assert_eq!(event.actor, "admin");
        assert_eq!(bps, 750);
    }

    #[test]
    fn rejected_fee_rate_change_emits_no_event() {
        let mut bps = INSURANCE_FEE_SHARE_BPS;

        let result = set_insurance_fee_share(&mut bps, BPS_DENOMINATOR + 1, "admin");

        assert_eq!(result, Err(ContractError::Unauthorized));
        assert_eq!(bps, INSURANCE_FEE_SHARE_BPS);
    }

    #[test]
    fn cap_change_emits_one_versioned_event() {
        let mut pool = pool(100, 100, 0);

        let event = set_insurance_cap(&mut pool, 5_000, "admin").unwrap();

        assert_eq!(event.field, ConfigField::Cap);
        assert_eq!(event.old_value, Some(100));
        assert_eq!(event.new_value, Some(5_000));
        assert_eq!(event.actor, "admin");
        assert_eq!(pool.balance, 5_000);
    }

    #[test]
    fn rejected_cap_change_emits_no_event() {
        let mut pool = InsurancePool { balance: 5_000 };

        let result = set_insurance_cap(&mut pool, 100, "admin");

        assert_eq!(result, Err(ContractError::Unauthorized));
        assert_eq!(pool.balance, 5_000);
    }

    #[test]
    fn withdrawal_at_exact_floor_is_allowed() {
        let mut pool = pool(1_000, 1_000, 400);

        let remaining = withdraw_protected_reserves(&mut pool, 600).unwrap();

        assert_eq!(remaining, 400);
        assert_eq!(pool.protected_reserves, 400);
        assert_eq!(pool.solvency_floor, 400);
    }

    #[test]
    fn withdrawal_below_floor_is_rejected_and_state_unchanged() {
        let mut pool = pool(1_000, 1_000, 400);

        let result = withdraw_protected_reserves(&mut pool, 601);

        assert_eq!(result, Err(ContractError::SolvencyFloorBreached));
        assert_eq!(pool.protected_reserves, 1_000);
        assert_eq!(pool.balance, 1_000);
    }

    #[test]
    fn withdrawal_allowed_after_reserves_replenished() {
        let mut pool = pool(1_000, 1_000, 400);

        // First withdrawal drains reserves down to the floor.
        assert_eq!(withdraw_protected_reserves(&mut pool, 600), Ok(400));
        // Further withdrawal is rejected at the floor.
        assert_eq!(
            withdraw_protected_reserves(&mut pool, 1),
            Err(ContractError::SolvencyFloorBreached)
        );

        // Replenish reserves above the floor.
        pool.protected_reserves += 500;

        // Withdrawal is allowed again.
        assert_eq!(withdraw_protected_reserves(&mut pool, 500), Ok(400));
        assert_eq!(pool.protected_reserves, 400);
    }

    #[test]
    fn claim_below_floor_is_rejected_and_state_unchanged() {
        let mut pool = pool(1_000, 500, 400);
        let trade = TradeLoss {
            trade_id: 45,
            slashed_signal: true,
            stop_loss_amount: 100,
        };

        // Half of 600 is 300, which would drop protected reserves to 200 < 400.
        let result = claim_insurance(&mut pool, "user-1", &trade, 600);

        assert_eq!(result, Err(ContractError::SolvencyFloorBreached));
        assert_eq!(pool.balance, 1_000);
        assert_eq!(pool.protected_reserves, 500);
    }

    #[test]
    fn dust_sweep_moves_sub_threshold_balance_and_records_audit() {
        let mut balance = 400;

        let event = sweep_dust(&mut balance, "dest-1", 400, true, "admin").unwrap();

        assert_eq!(event.schema_version, CONFIG_EVENT_SCHEMA_VERSION);
        assert_eq!(event.topic, DUST_SWEPT_TOPIC);
        assert_eq!(event.destination, "dest-1");
        assert_eq!(event.amount, 400);
        assert_eq!(event.actor, "admin");
        assert_eq!(balance, 0);
    }

    #[test]
    fn dust_sweep_rejects_unauthorized_caller() {
        let mut balance = 400;

        let result = sweep_dust(&mut balance, "dest-1", 400, false, "attacker");

        assert_eq!(result, Err(ContractError::Unauthorized));
        assert_eq!(balance, 400);
    }

    #[test]
    fn dust_sweep_rejects_ordinary_fee_balance() {
        let mut balance = DUST_SWEEP_THRESHOLD;

        let result = sweep_dust(&mut balance, "dest-1", 1, true, "admin");

        assert_eq!(result, Err(ContractError::InvalidSweep));
        assert_eq!(balance, DUST_SWEEP_THRESHOLD);
    }

    #[test]
    fn dust_sweep_rejects_invalid_amount() {
        let mut balance = 400;

        assert_eq!(
            sweep_dust(&mut balance, "dest-1", 0, true, "admin"),
            Err(ContractError::InvalidSweep)
        );
        assert_eq!(
            sweep_dust(&mut balance, "dest-1", 401, true, "admin"),
            Err(ContractError::InvalidSweep)
        );
        assert_eq!(balance, 400);
    }
    }
}
