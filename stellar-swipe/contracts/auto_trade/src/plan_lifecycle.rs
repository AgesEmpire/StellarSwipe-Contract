//! Auto-trade plan lifecycle: activation guards (#1145), drawdown recovery
//! cooldown (#1147), keeper execution leases (#1148) and DCA schedule drift
//! detection (#1150).

use soroban_sdk::{contracttype, symbol_short, token, Address, Env};

use crate::errors::AutoTradeError;

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStatus {
    Draft,
    Active,
    /// Paused by the drawdown circuit breaker; cooldown applies.
    Tripped,
    /// Manual emergency disable; overrides any activation path.
    EmergencyDisabled,
}

/// Typed reason emitted when activation is rejected.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationReject {
    AssetUnsupported,
    OracleUnavailable,
    RiskLimitExceeded,
    InsufficientFunding,
}

/// Policy applied when ledger time skipped one or more DCA intervals.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatchUpPolicy {
    /// Missed intervals are dropped; a single execution realigns the schedule.
    Skip,
    /// Missed intervals are folded into one execution.
    Coalesce,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoTradePlan {
    pub owner: Address,
    pub asset_pair: u32,
    pub funding_token: Address,
    pub budget: i128,
    /// Fraction of portfolio the plan may commit (0-100), checked against risk config.
    pub position_pct: u32,
    pub status: PlanStatus,
    /// DCA interval in seconds (0 = not a DCA plan).
    pub dca_interval: u64,
    /// Timestamp of the next scheduled DCA execution.
    pub next_dca_at: u64,
    pub cooldown_start_ledger: u32,
    pub cooldown_release_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeeperLease {
    pub keeper: Address,
    pub expires_ledger: u32,
    /// Monotonic per-plan epoch; bumps on every (re)acquisition so stale
    /// keepers holding an older epoch cannot finalize work.
    pub epoch: u64,
}

/// Outcome of a DCA drift evaluation.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriftDecision {
    /// Intervals that were due (scheduled) at evaluation time.
    pub scheduled_intervals: u64,
    /// Intervals actually executed by this call (0 or 1 for Skip, 1 for Coalesce).
    pub executed_intervals: u64,
    /// Intervals beyond the first that ledger time skipped.
    pub missed_intervals: u64,
    pub policy: CatchUpPolicy,
    pub next_dca_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum PlanKey {
    NextId,
    Plan(u64),
    SupportedAsset(u32),
    RecoveryCooldownLedgers,
    Lease(u64),
    LeaseEpoch(u64),
    CatchUpPolicy(u64),
}

pub const DEFAULT_RECOVERY_COOLDOWN_LEDGERS: u32 = 720; // ~1h at 5s ledgers
pub const MAX_LEASE_LEDGERS: u32 = 1_000;

// ── Storage helpers ───────────────────────────────────────────────────────────

pub fn get_plan(env: &Env, plan_id: u64) -> Result<AutoTradePlan, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&PlanKey::Plan(plan_id))
        .ok_or(AutoTradeError::PlanNotFound)
}

fn save_plan(env: &Env, plan_id: u64, plan: &AutoTradePlan) {
    env.storage().persistent().set(&PlanKey::Plan(plan_id), plan);
}

fn require_owner(plan: &AutoTradePlan, caller: &Address) -> Result<(), AutoTradeError> {
    caller.require_auth();
    if &plan.owner != caller {
        return Err(AutoTradeError::Unauthorized);
    }
    Ok(())
}

// ── Plan creation / admin config ──────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn create_plan(
    env: &Env,
    owner: Address,
    asset_pair: u32,
    funding_token: Address,
    budget: i128,
    position_pct: u32,
    dca_interval: u64,
) -> Result<u64, AutoTradeError> {
    owner.require_auth();
    if budget <= 0 || position_pct > 100 {
        return Err(AutoTradeError::InvalidAmount);
    }
    let id: u64 = env.storage().instance().get(&PlanKey::NextId).unwrap_or(0);
    let next = id.checked_add(1).ok_or(AutoTradeError::ScheduleOverflow)?;
    env.storage().instance().set(&PlanKey::NextId, &next);
    let plan = AutoTradePlan {
        owner,
        asset_pair,
        funding_token,
        budget,
        position_pct,
        status: PlanStatus::Draft,
        dca_interval,
        next_dca_at: 0,
        cooldown_start_ledger: 0,
        cooldown_release_ledger: 0,
    };
    save_plan(env, id, &plan);
    env.events().publish((symbol_short!("plan_new"), id), plan.owner);
    Ok(id)
}

pub fn set_supported_asset(
    env: &Env,
    admin: &Address,
    asset_pair: u32,
    supported: bool,
) -> Result<(), AutoTradeError> {
    crate::admin::require_admin(env, admin)?;
    env.storage()
        .persistent()
        .set(&PlanKey::SupportedAsset(asset_pair), &supported);
    Ok(())
}

pub fn set_recovery_cooldown(env: &Env, admin: &Address, ledgers: u32) -> Result<(), AutoTradeError> {
    crate::admin::require_admin(env, admin)?;
    if ledgers == 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    env.storage()
        .instance()
        .set(&PlanKey::RecoveryCooldownLedgers, &ledgers);
    Ok(())
}

fn recovery_cooldown(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&PlanKey::RecoveryCooldownLedgers)
        .unwrap_or(DEFAULT_RECOVERY_COOLDOWN_LEDGERS)
}

// ── #1145 Activation guards ───────────────────────────────────────────────────

/// Pure prerequisite evaluation; performs no writes so a failure leaves the
/// draft untouched.
pub fn check_activation(env: &Env, plan: &AutoTradePlan) -> Result<(), ActivationReject> {
    let supported: bool = env
        .storage()
        .persistent()
        .get(&PlanKey::SupportedAsset(plan.asset_pair))
        .unwrap_or(false);
    if !supported {
        return Err(ActivationReject::AssetUnsupported);
    }
    if crate::oracle::check_oracle_circuit_breaker(env, plan.asset_pair).is_err()
        || crate::oracle::get_aggregated_price(env, plan.asset_pair).is_err()
    {
        return Err(ActivationReject::OracleUnavailable);
    }
    let risk = crate::risk::get_risk_config(env, &plan.owner);
    if plan.position_pct > risk.max_position_pct {
        return Err(ActivationReject::RiskLimitExceeded);
    }
    let balance = token::Client::new(env, &plan.funding_token).balance(&plan.owner);
    if balance < plan.budget {
        return Err(ActivationReject::InsufficientFunding);
    }
    Ok(())
}

/// Transition Draft → Active (or Tripped → Active once the cooldown has
/// released). All checks run before any write.
pub fn activate_plan(env: &Env, caller: &Address, plan_id: u64) -> Result<(), AutoTradeError> {
    let mut plan = get_plan(env, plan_id)?;
    require_owner(&plan, caller)?;
    match plan.status {
        PlanStatus::Draft => {}
        PlanStatus::Tripped => {
            if env.ledger().sequence() < plan.cooldown_release_ledger {
                return Err(AutoTradeError::RecoveryCooldownActive);
            }
        }
        PlanStatus::EmergencyDisabled => return Err(AutoTradeError::PlanDisabled),
        PlanStatus::Active => return Err(AutoTradeError::InvalidPlanState),
    }
    if let Err(reason) = check_activation(env, &plan) {
        env.events()
            .publish((symbol_short!("act_rej"), plan_id), reason);
        return Err(AutoTradeError::ActivationPrerequisiteFailed);
    }
    let was_tripped = plan.status == PlanStatus::Tripped;
    plan.status = PlanStatus::Active;
    if plan.dca_interval > 0 && plan.next_dca_at == 0 {
        plan.next_dca_at = env
            .ledger()
            .timestamp()
            .checked_add(plan.dca_interval)
            .ok_or(AutoTradeError::ScheduleOverflow)?;
    }
    save_plan(env, plan_id, &plan);
    if was_tripped {
        env.events().publish(
            (symbol_short!("rcv_rel"), plan_id),
            (plan.cooldown_start_ledger, plan.cooldown_release_ledger),
        );
    }
    env.events().publish((symbol_short!("plan_act"), plan_id), ());
    Ok(())
}

// ── #1147 Drawdown recovery cooldown ──────────────────────────────────────────

/// Called when the drawdown circuit breaker fires for this plan's owner.
/// Idempotent: a plan already tripped keeps its original cooldown window and
/// no duplicate event is emitted.
pub fn trip_drawdown(env: &Env, plan_id: u64) -> Result<(), AutoTradeError> {
    let mut plan = get_plan(env, plan_id)?;
    if plan.status != PlanStatus::Active {
        return Ok(());
    }
    let start = env.ledger().sequence();
    let release = start
        .checked_add(recovery_cooldown(env))
        .ok_or(AutoTradeError::ScheduleOverflow)?;
    plan.status = PlanStatus::Tripped;
    plan.cooldown_start_ledger = start;
    plan.cooldown_release_ledger = release;
    save_plan(env, plan_id, &plan);
    env.events()
        .publish((symbol_short!("rcv_start"), plan_id), (start, release));
    Ok(())
}

/// Evaluate the owner's drawdown trigger and trip the plan if breached.
pub fn check_plan_drawdown(env: &Env, plan_id: u64, current_value: i128) -> Result<bool, AutoTradeError> {
    let plan = get_plan(env, plan_id)?;
    let breached = crate::drawdown::check_drawdown_trigger(env, &plan.owner, current_value);
    if breached {
        trip_drawdown(env, plan_id)?;
    }
    Ok(breached)
}

/// Manual emergency disable (owner or admin). Overrides activation until
/// explicitly cleared by the admin.
pub fn emergency_disable(env: &Env, caller: &Address, plan_id: u64) -> Result<(), AutoTradeError> {
    let mut plan = get_plan(env, plan_id)?;
    if require_owner(&plan, caller).is_err() {
        crate::admin::require_admin(env, caller)?;
    }
    if plan.status == PlanStatus::EmergencyDisabled {
        return Ok(());
    }
    plan.status = PlanStatus::EmergencyDisabled;
    save_plan(env, plan_id, &plan);
    env.storage().persistent().remove(&PlanKey::Lease(plan_id));
    env.events().publish((symbol_short!("plan_dis"), plan_id), ());
    Ok(())
}

/// Admin clears an emergency disable; the plan returns to Draft and must pass
/// activation guards again.
pub fn clear_emergency_disable(env: &Env, admin: &Address, plan_id: u64) -> Result<(), AutoTradeError> {
    crate::admin::require_admin(env, admin)?;
    let mut plan = get_plan(env, plan_id)?;
    if plan.status != PlanStatus::EmergencyDisabled {
        return Err(AutoTradeError::InvalidPlanState);
    }
    plan.status = PlanStatus::Draft;
    save_plan(env, plan_id, &plan);
    Ok(())
}

// ── #1148 Keeper lease expiry ─────────────────────────────────────────────────

pub fn get_lease(env: &Env, plan_id: u64) -> Option<KeeperLease> {
    env.storage().persistent().get(&PlanKey::Lease(plan_id))
}

fn lease_live(env: &Env, lease: &KeeperLease) -> bool {
    env.ledger().sequence() < lease.expires_ledger
}

/// Acquire (or reclaim an expired) lease. Returns the new lease epoch.
pub fn acquire_lease(
    env: &Env,
    keeper: &Address,
    plan_id: u64,
    duration_ledgers: u32,
) -> Result<u64, AutoTradeError> {
    keeper.require_auth();
    crate::keeper::require_registered_keeper(env, keeper)?;
    if duration_ledgers == 0 || duration_ledgers > MAX_LEASE_LEDGERS {
        return Err(AutoTradeError::InvalidAmount);
    }
    let plan = get_plan(env, plan_id)?;
    if plan.status != PlanStatus::Active {
        return Err(AutoTradeError::InvalidPlanState);
    }
    if let Some(existing) = get_lease(env, plan_id) {
        if lease_live(env, &existing) && &existing.keeper != keeper {
            return Err(AutoTradeError::LeaseHeld);
        }
    }
    let epoch: u64 = env
        .storage()
        .persistent()
        .get(&PlanKey::LeaseEpoch(plan_id))
        .unwrap_or(0u64)
        .checked_add(1)
        .ok_or(AutoTradeError::ScheduleOverflow)?;
    let expires_ledger = env
        .ledger()
        .sequence()
        .checked_add(duration_ledgers)
        .ok_or(AutoTradeError::ScheduleOverflow)?;
    let lease = KeeperLease { keeper: keeper.clone(), expires_ledger, epoch };
    env.storage().persistent().set(&PlanKey::LeaseEpoch(plan_id), &epoch);
    env.storage().persistent().set(&PlanKey::Lease(plan_id), &lease);
    env.events()
        .publish((symbol_short!("lease_acq"), plan_id), (keeper.clone(), epoch, expires_ledger));
    Ok(epoch)
}

/// Extend a live lease. Renewal at or after expiry is rejected so a lapsed
/// keeper must re-acquire (and may lose the race to another keeper).
pub fn renew_lease(
    env: &Env,
    keeper: &Address,
    plan_id: u64,
    epoch: u64,
    duration_ledgers: u32,
) -> Result<u32, AutoTradeError> {
    keeper.require_auth();
    crate::keeper::require_registered_keeper(env, keeper)?;
    if duration_ledgers == 0 || duration_ledgers > MAX_LEASE_LEDGERS {
        return Err(AutoTradeError::InvalidAmount);
    }
    let mut lease = require_live_lease(env, keeper, plan_id, epoch)?;
    lease.expires_ledger = env
        .ledger()
        .sequence()
        .checked_add(duration_ledgers)
        .ok_or(AutoTradeError::ScheduleOverflow)?;
    env.storage().persistent().set(&PlanKey::Lease(plan_id), &lease);
    env.events()
        .publish((symbol_short!("lease_rnw"), plan_id), (epoch, lease.expires_ledger));
    Ok(lease.expires_ledger)
}

fn require_live_lease(
    env: &Env,
    keeper: &Address,
    plan_id: u64,
    epoch: u64,
) -> Result<KeeperLease, AutoTradeError> {
    let lease = get_lease(env, plan_id).ok_or(AutoTradeError::LeaseExpired)?;
    if &lease.keeper != keeper || lease.epoch != epoch {
        return Err(AutoTradeError::Unauthorized);
    }
    if !lease_live(env, &lease) {
        return Err(AutoTradeError::LeaseExpired);
    }
    Ok(lease)
}

/// Finalize keeper work: requires the caller's lease to still be live and
/// current. Releases the lease on success.
pub fn finalize_with_lease(
    env: &Env,
    keeper: &Address,
    plan_id: u64,
    epoch: u64,
) -> Result<(), AutoTradeError> {
    keeper.require_auth();
    require_live_lease(env, keeper, plan_id, epoch)?;
    env.storage().persistent().remove(&PlanKey::Lease(plan_id));
    env.events().publish((symbol_short!("lease_fin"), plan_id), epoch);
    Ok(())
}

// ── #1150 DCA schedule drift detection ────────────────────────────────────────

/// Set the plan's catch-up policy. Only one policy may ever apply per plan.
pub fn set_catch_up_policy(
    env: &Env,
    caller: &Address,
    plan_id: u64,
    policy: CatchUpPolicy,
) -> Result<(), AutoTradeError> {
    let plan = get_plan(env, plan_id)?;
    require_owner(&plan, caller)?;
    let key = PlanKey::CatchUpPolicy(plan_id);
    if env.storage().persistent().has(&key) {
        return Err(AutoTradeError::CatchUpPolicyAlreadySet);
    }
    env.storage().persistent().set(&key, &policy);
    Ok(())
}

pub fn get_catch_up_policy(env: &Env, plan_id: u64) -> CatchUpPolicy {
    env.storage()
        .persistent()
        .get(&PlanKey::CatchUpPolicy(plan_id))
        .unwrap_or(CatchUpPolicy::Skip)
}

/// Pure, deterministic drift computation using checked arithmetic.
/// Returns `None` when no interval is due yet.
pub fn compute_drift(
    now: u64,
    next_dca_at: u64,
    interval: u64,
    policy: CatchUpPolicy,
) -> Result<Option<DriftDecision>, AutoTradeError> {
    if interval == 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    if now < next_dca_at {
        return Ok(None);
    }
    let elapsed = now - next_dca_at;
    let scheduled = (elapsed / interval)
        .checked_add(1)
        .ok_or(AutoTradeError::ScheduleOverflow)?;
    let advance = scheduled
        .checked_mul(interval)
        .ok_or(AutoTradeError::ScheduleOverflow)?;
    let next = next_dca_at
        .checked_add(advance)
        .ok_or(AutoTradeError::ScheduleOverflow)?;
    let executed = match policy {
        CatchUpPolicy::Skip => 1,
        CatchUpPolicy::Coalesce => scheduled,
    };
    Ok(Some(DriftDecision {
        scheduled_intervals: scheduled,
        executed_intervals: executed,
        missed_intervals: scheduled - 1,
        policy,
        next_dca_at: next,
    }))
}

/// Evaluate drift for an active DCA plan, advance its schedule, and emit
/// scheduled vs. executed intervals. Requires a live keeper lease.
pub fn process_dca_tick(
    env: &Env,
    keeper: &Address,
    plan_id: u64,
    epoch: u64,
) -> Result<Option<DriftDecision>, AutoTradeError> {
    keeper.require_auth();
    require_live_lease(env, keeper, plan_id, epoch)?;
    let mut plan = get_plan(env, plan_id)?;
    if plan.status != PlanStatus::Active {
        return Err(AutoTradeError::InvalidPlanState);
    }
    let policy = get_catch_up_policy(env, plan_id);
    let decision = compute_drift(env.ledger().timestamp(), plan.next_dca_at, plan.dca_interval, policy)?;
    if let Some(d) = &decision {
        plan.next_dca_at = d.next_dca_at;
        save_plan(env, plan_id, &plan);
        if d.missed_intervals > 0 {
            env.events().publish(
                (symbol_short!("dca_drift"), plan_id),
                (d.scheduled_intervals, d.executed_intervals, d.missed_intervals, d.policy),
            );
        }
        env.events().publish(
            (symbol_short!("dca_exec"), plan_id),
            (d.scheduled_intervals, d.executed_intervals, d.next_dca_at),
        );
    }
    Ok(decision)
}
