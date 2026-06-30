use soroban_sdk::{Address, Env, IntoVal, Symbol};
use stellar_swipe_common::{checked_amount::Amount, Asset};

use crate::storage::{
    get_fee_rate, get_monthly_trade_volume, get_oracle_contract, get_volume_discount_config,
    remove_monthly_trade_volume, set_monthly_trade_volume, MonthlyTradeVolume, GOLD_DISCOUNT_BPS,
    GOLD_TIER_VOLUME_USD, LEDGERS_PER_MONTH_APPROX, MIN_FEE_RATE_BPS, SILVER_DISCOUNT_BPS,
    SILVER_TIER_VOLUME_USD,
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
