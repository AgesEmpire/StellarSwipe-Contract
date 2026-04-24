use soroban_sdk::{Address, Env, IntoVal, Symbol};
use stellar_swipe_common::Asset;

use crate::storage::{
    get_fee_rate, get_monthly_trade_volume, get_oracle_contract, remove_monthly_trade_volume,
    set_monthly_trade_volume, MonthlyTradeVolume, GOLD_DISCOUNT_BPS, GOLD_TIER_VOLUME_USD,
    LEDGERS_PER_MONTH_APPROX, MIN_FEE_RATE_BPS, SILVER_DISCOUNT_BPS, SILVER_TIER_VOLUME_USD,
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
    let base_rate = get_fee_rate(env);
    let volume_usd = get_active_volume_usd(env, user);

    if volume_usd >= GOLD_TIER_VOLUME_USD {
        base_rate
            .saturating_sub(GOLD_DISCOUNT_BPS)
            .max(MIN_FEE_RATE_BPS)
    } else if volume_usd >= SILVER_TIER_VOLUME_USD {
        base_rate
            .saturating_sub(SILVER_DISCOUNT_BPS)
            .max(MIN_FEE_RATE_BPS)
    } else {
        base_rate
    }
}

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

    let updated_volume = current_volume
        .volume_usd
        .checked_add(usd_volume)
        .ok_or(ContractError::ArithmeticOverflow)?;

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
