#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Symbol, String, Vec};
use crate::monitoring::{get_bridge_transfer, TransferStatus};
use crate::governance::{get_bridge_validators};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeFeeConfig {
    pub bridge_id: u64,
    pub base_fee_bps: u32,
    pub min_fee: i128,
    pub max_fee: i128,
    pub validator_reward_pct: u32,
    pub treasury_pct: u32,
    pub dynamic_adjustment_enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeFeeStats {
    pub total_fees_collected: i128,
    pub fees_distributed_validators: i128,
    pub fees_to_treasury: i128,
    pub transfers_count: u64,
    pub avg_fee: i128,
}

#[contracttype]
pub enum FeeStorageKey {
    FeeConfig(u64),
    FeeStats(u64),
    TreasuryTarget(u64),
    DailyTransfers(u64),
}

pub fn set_bridge_treasury(env: &Env, bridge_id: u64, treasury: Address) {
    env.storage().persistent().set(&FeeStorageKey::TreasuryTarget(bridge_id), &treasury);
}

pub fn get_bridge_treasury(env: &Env, bridge_id: u64) -> Result<Address, String> {
    env.storage().persistent().get(&FeeStorageKey::TreasuryTarget(bridge_id))
        .ok_or_else(|| String::from_str(env, "Treasury not set"))
}

pub fn set_bridge_fee_config(env: &Env, config: &BridgeFeeConfig) {
    env.storage().persistent().set(&FeeStorageKey::FeeConfig(config.bridge_id), config);
}

pub fn get_bridge_fee_config(env: &Env, bridge_id: u64) -> Result<BridgeFeeConfig, String> {
    env.storage().persistent().get(&FeeStorageKey::FeeConfig(bridge_id))
        .ok_or_else(|| String::from_str(env, "Fee config not found"))
}

pub fn get_bridge_fee_stats(env: &Env, bridge_id: u64) -> BridgeFeeStats {
    env.storage().persistent().get(&FeeStorageKey::FeeStats(bridge_id))
        .unwrap_or(BridgeFeeStats {
            total_fees_collected: 0,
            fees_distributed_validators: 0,
            fees_to_treasury: 0,
            transfers_count: 0,
            avg_fee: 0,
        })
}

pub fn save_bridge_fee_stats(env: &Env, bridge_id: u64, stats: &BridgeFeeStats) {
    env.storage().persistent().set(&FeeStorageKey::FeeStats(bridge_id), stats);
}

pub fn calculate_bridge_fee(env: &Env, bridge_id: u64, transfer_amount: i128) -> Result<i128, String> {
    let fee_config = get_bridge_fee_config(env, bridge_id)?;

    let fee = (transfer_amount * fee_config.base_fee_bps as i128) / 10000;

    let bounded_fee = if fee < fee_config.min_fee {
        fee_config.min_fee
    } else if fee > fee_config.max_fee {
        fee_config.max_fee
    } else {
        fee
    };

    Ok(bounded_fee)
}

pub fn collect_bridge_fee(
    env: &Env,
    transfer_id: u64,
    user: Address,
    amount: i128,
) -> Result<i128, String> {
    let transfer = get_bridge_transfer(env, transfer_id)
        .ok_or_else(|| String::from_str(env, "Transfer not found"))?;
    
    let fee = calculate_bridge_fee(env, transfer.bridge_id, amount)?;
    let net_amount = amount - fee;

    let mut stats = get_bridge_fee_stats(env, transfer.bridge_id);
    let total_fees = stats.total_fees_collected + fee;
    stats.transfers_count += 1;
    stats.total_fees_collected = total_fees;
    
    if stats.transfers_count > 0 {
        stats.avg_fee = total_fees / stats.transfers_count as i128;
    }
    
    save_bridge_fee_stats(env, transfer.bridge_id, &stats);

    let daily_transfers: u64 = env.storage().persistent().get(&FeeStorageKey::DailyTransfers(transfer.bridge_id)).unwrap_or(0);
    env.storage().persistent().set(&FeeStorageKey::DailyTransfers(transfer.bridge_id), &(daily_transfers + 1));

    env.events().publish(
        (Symbol::new(env, "bridge_fee_collected"), transfer_id),
        (user, fee, amount, net_amount),
    );

    Ok(net_amount)
}

pub fn distribute_validator_rewards(env: &Env, bridge_id: u64) -> Result<(), String> {
    let fee_config = get_bridge_fee_config(env, bridge_id)?;
    let mut fee_stats = get_bridge_fee_stats(env, bridge_id);

    let target_validator_total = (fee_stats.total_fees_collected * fee_config.validator_reward_pct as i128) / 10000;
    let validator_share = target_validator_total - fee_stats.fees_distributed_validators;

    if validator_share <= 0 {
        return Ok(());
    }

    let validators: Vec<Address> = get_bridge_validators(env, bridge_id)?;
    if validators.is_empty() {
        return Err(String::from_str(env, "No validators found"));
    }

    let per_validator = validator_share / validators.len() as i128;

    for i in 0..validators.len() {
        let validator: Address = validators.get(i).unwrap();
        env.events().publish(
            (Symbol::new(env, "validator_reward_dist"), bridge_id),
            (validator, per_validator),
        );
    }

    fee_stats.fees_distributed_validators += validator_share;
    save_bridge_fee_stats(env, bridge_id, &fee_stats);

    Ok(())
}

pub fn allocate_to_treasury(env: &Env, bridge_id: u64) -> Result<(), String> {
    let fee_config = get_bridge_fee_config(env, bridge_id)?;
    let mut fee_stats = get_bridge_fee_stats(env, bridge_id);

    let target_treasury_total = (fee_stats.total_fees_collected * fee_config.treasury_pct as i128) / 10000;
    let treasury_share = target_treasury_total - fee_stats.fees_to_treasury;

    if treasury_share <= 0 {
        return Ok(());
    }

    let _treasury_address = get_bridge_treasury(env, bridge_id)?;
    
    fee_stats.fees_to_treasury += treasury_share;
    save_bridge_fee_stats(env, bridge_id, &fee_stats);

    env.events().publish(
        (Symbol::new(env, "treasury_allocation"), bridge_id),
        treasury_share,
    );

    Ok(())
}

fn min(a: u32, b: u32) -> u32 { if a < b { a } else { b } }
fn max(a: u32, b: u32) -> u32 { if a > b { a } else { b } }

pub fn adjust_bridge_fees_dynamically(env: &Env, bridge_id: u64) -> Result<(), String> {
    let mut fee_config = get_bridge_fee_config(env, bridge_id)?;

    if !fee_config.dynamic_adjustment_enabled {
        return Ok(());
    }

    let utilization = calculate_bridge_utilization(env, bridge_id)?;

    match utilization {
        0..=3000 => {
            fee_config.base_fee_bps = max(10, fee_config.base_fee_bps.saturating_sub(5));
        },
        7000..=10000 => {
            fee_config.base_fee_bps = min(100, fee_config.base_fee_bps + 5);
        },
        _ => {}
    }

    set_bridge_fee_config(env, &fee_config);

    env.events().publish(
        (Symbol::new(env, "bridge_fees_adjusted"), bridge_id),
        (fee_config.base_fee_bps, utilization),
    );

    Ok(())
}

fn get_bridge_max_capacity(_env: &Env, _bridge_id: u64) -> Result<u64, String> {
    Ok(10000)
}

pub fn calculate_bridge_utilization(env: &Env, bridge_id: u64) -> Result<u32, String> {
    let transfers_24h: u64 = env.storage().persistent().get(&FeeStorageKey::DailyTransfers(bridge_id)).unwrap_or(0);
    let max_capacity = get_bridge_max_capacity(env, bridge_id)?;

    let max_cap = if max_capacity == 0 { 1 } else { max_capacity };
    let utilization_bps = (transfers_24h * 10000) / max_cap;
    let res = if utilization_bps > 10000 { 10000 } else { utilization_bps as u32 };
    Ok(res)
}

pub fn refund_bridge_fee(env: &Env, transfer_id: u64, reason: String) -> Result<(), String> {
    let transfer = get_bridge_transfer(env, transfer_id)
        .ok_or_else(|| String::from_str(env, "Transfer not found"))?;

    // We assume the caller checked if transfer status is Failed, since TransferStatus does not have Cancelled.
    if transfer.status != TransferStatus::Failed {
        return Err(String::from_str(env, "Only failed transfers eligible for refund"));
    }

    let mut fee_stats = get_bridge_fee_stats(env, transfer.bridge_id);
    fee_stats.total_fees_collected -= transfer.fee_paid;
    save_bridge_fee_stats(env, transfer.bridge_id, &fee_stats);

    env.events().publish(
        (Symbol::new(env, "bridge_fee_refunded"), transfer_id),
        (transfer.user, transfer.fee_paid, reason),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_fee_calculations() {
        let env = Env::default();
        let bridge_id = 1;

        let config = BridgeFeeConfig {
            bridge_id,
            base_fee_bps: 30, // 0.3%
            min_fee: 100,
            max_fee: 10000,
            validator_reward_pct: 8000, // 80%
            treasury_pct: 2000,         // 20%
            dynamic_adjustment_enabled: true,
        };
        set_bridge_fee_config(&env, &config);

        // 0.3% of 1000 is 3, but min_fee is 100
        let fee1 = calculate_bridge_fee(&env, bridge_id, 1000).unwrap();
        assert_eq!(fee1, 100);

        // 0.3% of 1,000,000 is 3000
        let fee2 = calculate_bridge_fee(&env, bridge_id, 1_000_000).unwrap();
        assert_eq!(fee2, 3000);

        // 0.3% of 10,000,000 is 30,000, but max_fee is 10000
        let fee3 = calculate_bridge_fee(&env, bridge_id, 10_000_000).unwrap();
        assert_eq!(fee3, 10000);
    }
}
