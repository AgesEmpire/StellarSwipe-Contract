use soroban_sdk::{Address, Env, IntoVal, Symbol};
use stellar_swipe_common::{checked_amount::Amount, Asset};

use crate::events::emit_rebate_cap_applied;
use crate::storage::{
    add_pending_rebate_claim, clear_pending_rebate_claims, get_admin, get_daily_fee_total,
    get_fee_rate, get_max_rebate_bps as get_max_rebate_bps_storage, get_monthly_trade_volume,
    get_oracle_contract, get_pending_fees, get_pending_rebate_claims, get_volume_discount_config,
    is_authorized_caller, remove_monthly_trade_volume, set_max_rebate_bps as set_max_rebate_bps_storage,
    set_monthly_trade_volume, set_pending_fees, MonthlyTradeVolume, GOLD_DISCOUNT_BPS,
    GOLD_TIER_VOLUME_USD, LEDGERS_PER_MONTH_APPROX, MIN_FEE_RATE_BPS, SECONDS_PER_DAY_FC,
    SILVER_DISCOUNT_BPS, SILVER_TIER_VOLUME_USD,
};
use crate::ContractError;

fn current_month_bucket(env: &Env) -> u32 {
    env.ledger().sequence() / LEDGERS_PER_MONTH_APPROX
}

fn active_monthly_trade_volume(env: &Env, user: &Address) -> Option<MonthlyTradeVolume> {
    let volume = get_monthly_trade_volume(env, user)?;
    if volume.month_bucket == current_month_bucket(env) {
        Some(volume)
    } else {
        remove_monthly_trade_volume(env, user);
        None
    }
}

pub fn get_active_volume_usd(env: &Env, user: &Address) -> i128 {
    active_monthly_trade_volume(env, user)
        .map(|volume| volume.volume_usd)
        .unwrap_or(0)
}

pub fn get_fee_rate_for_user(env: &Env, user: &Address) -> u32 {
    let global_base_rate = get_fee_rate(env);
    let congestion_multiplier = crate::FeeCollector::current_effective_multiplier(env);

    // Documented order of operations:
    // base fee × congestion multiplier → tiered volume discount → final fee

    // 1. base fee * congestion multiplier (multiplier 10_000 = 1.0x)
    let adjusted_base_rate = (global_base_rate as u64)
        .saturating_mul(congestion_multiplier as u64)
        .checked_div(10_000)
        .unwrap_or(global_base_rate as u64) as u32;

    let volume_usd = get_active_volume_usd(env, user);

    // 2. apply tiered volume discount
    // Admin-configured tiers take precedence over hardcoded defaults (#664).
    if let Some(config) = get_volume_discount_config(env) {
        let mut best_discount: u32 = 0;
        for i in 0..config.tiers.len() {
            let tier = config.tiers.get(i).unwrap();
            if volume_usd >= tier.volume_threshold_usd && tier.discount_bps > best_discount {
                best_discount = tier.discount_bps;
            }
        }
        return adjusted_base_rate
            .saturating_sub(best_discount)
            .max(MIN_FEE_RATE_BPS);
    }

    // Fallback: hardcoded two-tier defaults.
    if volume_usd >= GOLD_TIER_VOLUME_USD {
        adjusted_base_rate
            .saturating_sub(GOLD_DISCOUNT_BPS)
            .max(MIN_FEE_RATE_BPS)
    } else if volume_usd >= SILVER_TIER_VOLUME_USD {
        adjusted_base_rate
            .saturating_sub(SILVER_DISCOUNT_BPS)
            .max(MIN_FEE_RATE_BPS)
    } else {
        adjusted_base_rate.max(MIN_FEE_RATE_BPS)
    }
}

/// All financial arithmetic in this function goes through `Amount`'s checked
/// methods; `clippy::arithmetic_side_effects` is set to warn (CI runs clippy
/// with `-D warnings`) to flag any future raw +/-/* (issue #599).
#[warn(clippy::arithmetic_side_effects)]
pub fn record_trade_volume(
    env: &Env,
    user: &Address,
    trade_asset: &Asset,
    amount: i128,
) -> Result<(), ContractError> {
    let oracle_contract = get_oracle_contract(env).ok_or(ContractError::OracleNotConfigured)?;
    let usd_volume = env
        .try_invoke_contract::<i128, soroban_sdk::Error>(
            &oracle_contract,
            &Symbol::new(env, "convert_to_base"),
            (&amount, trade_asset).into_val(env),
        )
        .map_err(|_| ContractError::OracleConversionFailed)?
        .map_err(|_| ContractError::OracleConversionFailed)?;

    let current_volume = active_monthly_trade_volume(env, user).unwrap_or(MonthlyTradeVolume {
        month_bucket: current_month_bucket(env),
        volume_usd: 0,
    });

    let updated_volume = Amount::new(current_volume.volume_usd)
        .checked_add(Amount::new(usd_volume))
        .map(Amount::value)
        .map_err(|_| ContractError::ArithmeticOverflow)?;

    set_monthly_trade_volume(
        env,
        user,
        &MonthlyTradeVolume {
            month_bucket: current_month_bucket(env),
            volume_usd: updated_volume,
        },
    );

    Ok(())
}

// ── Issue #799: Rebate Cap ───────────────────────────────────────────────────
//
// Providers accrue rebates via `submit_rebate_claim`, deposited by the admin
// or an authorized settlement caller. `distribute_rebates` then settles all
// claims pending for the current epoch (a day, matching the bucketing already
// used for `DailyFeeTotal`): if their sum exceeds `max_rebate_bps` of that
// epoch's collected fees, every claim is scaled down proportionally so the
// total distributed never exceeds the cap, closing the drain vector where a
// large or colluding claim set could otherwise pay out more than the
// fee_collector actually collected.

fn current_epoch(env: &Env) -> u64 {
    env.ledger().timestamp() / SECONDS_PER_DAY_FC
}

fn require_admin_or_authorized(env: &Env, caller: &Address) -> Result<(), ContractError> {
    caller.require_auth();
    if *caller != get_admin(env) && !is_authorized_caller(env, caller) {
        return Err(ContractError::UnauthorizedCaller);
    }
    Ok(())
}

pub fn get_max_rebate_bps(env: &Env) -> u32 {
    get_max_rebate_bps_storage(env)
}

/// Admin-only: configure the rebate cap, in basis points of epoch fees
/// (10_000 = 100%).
pub fn set_max_rebate_bps(env: &Env, caller: &Address, bps: u32) -> Result<(), ContractError> {
    caller.require_auth();
    if *caller != get_admin(env) {
        return Err(ContractError::Unauthorized);
    }
    if bps > 10_000 {
        return Err(ContractError::InvalidAmount);
    }
    set_max_rebate_bps_storage(env, bps);
    Ok(())
}

/// Record a pending rebate claim for `provider`, settled by the next
/// `distribute_rebates` call for the current epoch. Callable by the admin or
/// an allowlisted authorized caller (Issue #813), e.g. a trusted
/// rewards/settlement contract.
pub fn submit_rebate_claim(
    env: &Env,
    caller: &Address,
    provider: &Address,
    token: &Address,
    amount: i128,
) -> Result<(), ContractError> {
    require_admin_or_authorized(env, caller)?;
    if amount <= 0 {
        return Err(ContractError::InvalidAmount);
    }
    add_pending_rebate_claim(env, token, current_epoch(env), provider, amount);
    Ok(())
}

/// Admin-only: settle all rebate claims pending for `token` in the current
/// epoch. If their sum exceeds `max_rebate_bps` of the epoch's collected
/// fees, every claim is scaled proportionally so the total distributed sums
/// to the cap and a `RebateCapApplied` event is emitted. Distributed amounts
/// are credited to each provider's pending fees, claimable via `claim_fees`.
/// Returns the total amount distributed.
pub fn distribute_rebates(env: &Env, caller: &Address, token: &Address) -> Result<i128, ContractError> {
    caller.require_auth();
    if *caller != get_admin(env) {
        return Err(ContractError::Unauthorized);
    }

    let epoch = current_epoch(env);
    let claims = get_pending_rebate_claims(env, token, epoch);
    if claims.is_empty() {
        return Ok(0);
    }

    let mut total_requested: i128 = 0;
    for i in 0..claims.len() {
        total_requested = total_requested
            .checked_add(claims.get(i).unwrap().amount)
            .ok_or(ContractError::ArithmeticOverflow)?;
    }

    let epoch_fees = get_daily_fee_total(env, token, epoch);
    let max_bps = get_max_rebate_bps(env);
    let cap = epoch_fees
        .checked_mul(max_bps as i128)
        .and_then(|v| v.checked_div(10_000))
        .ok_or(ContractError::ArithmeticOverflow)?;

    let mut total_distributed: i128 = 0;
    let capped = total_requested > cap;
    for i in 0..claims.len() {
        let claim = claims.get(i).unwrap();
        let payout = if capped {
            claim
                .amount
                .checked_mul(cap)
                .and_then(|v| v.checked_div(total_requested))
                .unwrap_or(0)
        } else {
            claim.amount
        };
        if payout > 0 {
            let current = get_pending_fees(env, &claim.provider, token);
            set_pending_fees(env, &claim.provider, token, current.saturating_add(payout));
            total_distributed = total_distributed.saturating_add(payout);
        }
    }

    if capped {
        emit_rebate_cap_applied(env, epoch, total_requested, total_distributed);
    }

    clear_pending_rebate_claims(env, token, epoch);
    Ok(total_distributed)
}
