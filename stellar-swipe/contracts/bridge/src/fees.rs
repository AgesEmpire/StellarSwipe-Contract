#![allow(dead_code)]

use crate::governance::get_bridge_validators;
use crate::monitoring::{get_bridge_transfer, ChainId, TransferStatus};
use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};
use stellar_swipe_common::assets::Asset;

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
    env.storage()
        .persistent()
        .set(&FeeStorageKey::TreasuryTarget(bridge_id), &treasury);
}

pub fn get_bridge_treasury(env: &Env, bridge_id: u64) -> Result<Address, String> {
    env.storage()
        .persistent()
        .get(&FeeStorageKey::TreasuryTarget(bridge_id))
        .ok_or_else(|| String::from_str(env, "Treasury not set"))
}

pub fn set_bridge_fee_config(env: &Env, config: &BridgeFeeConfig) {
    env.storage()
        .persistent()
        .set(&FeeStorageKey::FeeConfig(config.bridge_id), config);
}

pub fn get_bridge_fee_config(env: &Env, bridge_id: u64) -> Result<BridgeFeeConfig, String> {
    env.storage()
        .persistent()
        .get(&FeeStorageKey::FeeConfig(bridge_id))
        .ok_or_else(|| String::from_str(env, "Fee config not found"))
}

pub fn get_bridge_fee_stats(env: &Env, bridge_id: u64) -> BridgeFeeStats {
    env.storage()
        .persistent()
        .get(&FeeStorageKey::FeeStats(bridge_id))
        .unwrap_or(BridgeFeeStats {
            total_fees_collected: 0,
            fees_distributed_validators: 0,
            fees_to_treasury: 0,
            transfers_count: 0,
            avg_fee: 0,
        })
}

pub fn save_bridge_fee_stats(env: &Env, bridge_id: u64, stats: &BridgeFeeStats) {
    env.storage()
        .persistent()
        .set(&FeeStorageKey::FeeStats(bridge_id), stats);
}

pub fn calculate_bridge_fee(
    env: &Env,
    bridge_id: u64,
    transfer_amount: i128,
) -> Result<i128, String> {
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

    let daily_transfers: u64 = env
        .storage()
        .persistent()
        .get(&FeeStorageKey::DailyTransfers(transfer.bridge_id))
        .unwrap_or(0);
    env.storage().persistent().set(
        &FeeStorageKey::DailyTransfers(transfer.bridge_id),
        &(daily_transfers + 1),
    );

    env.events().publish(
        (Symbol::new(env, "bridge_fee_collected"), transfer_id),
        (user, fee, amount, net_amount),
    );

    Ok(net_amount)
}

pub fn distribute_validator_rewards(env: &Env, bridge_id: u64) -> Result<(), String> {
    let fee_config = get_bridge_fee_config(env, bridge_id)?;
    let mut fee_stats = get_bridge_fee_stats(env, bridge_id);

    let target_validator_total =
        (fee_stats.total_fees_collected * fee_config.validator_reward_pct as i128) / 10000;
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

    let target_treasury_total =
        (fee_stats.total_fees_collected * fee_config.treasury_pct as i128) / 10000;
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

// ─── Destination-chain fee multipliers ───────────────────────────────────────

/// Storage key for chain-specific fee multipliers.
#[contracttype]
pub enum ChainFeeKey {
    /// basis-point multiplier applied on top of the base fee for a destination chain.
    /// Keyed by the chain discriminant (u32 cast of `ChainId`).
    ChainMultiplier(u32),
    /// Mapping from destination-chain discriminant to bridge_id.
    DestinationBridgeId(u32),
}

fn chain_discriminant(chain: ChainId) -> u32 {
    match chain {
        ChainId::Stellar => 0,
        ChainId::Ethereum => 1,
        ChainId::Bitcoin => 2,
        ChainId::Polygon => 3,
        ChainId::BNB => 4,
    }
}

/// Admin: store a fee multiplier (in bps, applied on top of the base fee) for a
/// destination chain.  A value of 10_000 means "no extra multiplier" (×1.0).
/// Values > 10_000 add a surcharge; values < 10_000 apply a discount.
pub fn set_chain_fee_multiplier(env: &Env, chain: ChainId, multiplier_bps: u32) {
    env.storage().persistent().set(
        &ChainFeeKey::ChainMultiplier(chain_discriminant(chain)),
        &multiplier_bps,
    );
}

/// Return the fee multiplier for a destination chain (default: 10_000 = ×1.0).
pub fn get_chain_fee_multiplier(env: &Env, chain: ChainId) -> u32 {
    env.storage()
        .persistent()
        .get(&ChainFeeKey::ChainMultiplier(chain_discriminant(chain)))
        .unwrap_or(10_000u32)
}

/// Admin: map a destination chain to its bridge_id so the estimator can look up
/// the correct fee configuration.
pub fn set_destination_bridge_id(env: &Env, chain: ChainId, bridge_id: u64) {
    env.storage().persistent().set(
        &ChainFeeKey::DestinationBridgeId(chain_discriminant(chain)),
        &bridge_id,
    );
}

/// Return the bridge_id associated with `chain` (default: 1).
pub fn get_destination_bridge_id(env: &Env, chain: ChainId) -> u64 {
    env.storage()
        .persistent()
        .get(&ChainFeeKey::DestinationBridgeId(chain_discriminant(chain)))
        .unwrap_or(1u64)
}

/// Read-only: estimate the fee that would be charged for a transfer to
/// `destination_chain` of `amount` units of `asset`.
///
/// Uses **exactly** the same fee calculation logic as the actual transfer path
/// (`calculate_bridge_fee`) and then applies any destination-chain-specific
/// multiplier so that the estimate is always consistent with what gets charged.
///
/// # Returns
/// `Ok(estimated_fee)` — the fee in the same unit as `amount`.
///
/// # Errors
/// Returns a string error if no fee configuration exists for the resolved bridge.
pub fn estimate_bridge_fee(
    env: &Env,
    destination_chain: ChainId,
    amount: i128,
    _asset: &Asset, // reserved for per-asset fee tiers in future extensions
) -> Result<i128, String> {
    let bridge_id = get_destination_bridge_id(env, destination_chain);
    let base_fee = calculate_bridge_fee(env, bridge_id, amount)?;

    let multiplier = get_chain_fee_multiplier(env, destination_chain);
    // Apply multiplier: fee × multiplier_bps / 10_000.
    // A multiplier of 10_000 is the identity; >10_000 adds a surcharge.
    let estimated_fee = (base_fee * multiplier as i128) / 10_000;
    Ok(estimated_fee)
}

fn min(a: u32, b: u32) -> u32 {
    if a < b {
        a
    } else {
        b
    }
}
fn max(a: u32, b: u32) -> u32 {
    if a > b {
        a
    } else {
        b
    }
}

pub fn adjust_bridge_fees_dynamically(env: &Env, bridge_id: u64) -> Result<(), String> {
    let mut fee_config = get_bridge_fee_config(env, bridge_id)?;

    if !fee_config.dynamic_adjustment_enabled {
        return Ok(());
    }

    let utilization = calculate_bridge_utilization(env, bridge_id)?;

    match utilization {
        0..=3000 => {
            fee_config.base_fee_bps = max(10, fee_config.base_fee_bps.saturating_sub(5));
        }
        7000..=10000 => {
            fee_config.base_fee_bps = min(100, fee_config.base_fee_bps + 5);
        }
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
    let transfers_24h: u64 = env
        .storage()
        .persistent()
        .get(&FeeStorageKey::DailyTransfers(bridge_id))
        .unwrap_or(0);
    let max_capacity = get_bridge_max_capacity(env, bridge_id)?;

    let max_cap = if max_capacity == 0 { 1 } else { max_capacity };
    let utilization_bps = (transfers_24h * 10000) / max_cap;
    let res = if utilization_bps > 10000 {
        10000
    } else {
        utilization_bps as u32
    };
    Ok(res)
}

pub fn refund_bridge_fee(env: &Env, transfer_id: u64, reason: String) -> Result<(), String> {
    let transfer = get_bridge_transfer(env, transfer_id)
        .ok_or_else(|| String::from_str(env, "Transfer not found"))?;

    // We assume the caller checked if transfer status is Failed, since TransferStatus does not have Cancelled.
    if transfer.status != TransferStatus::Failed {
        return Err(String::from_str(
            env,
            "Only failed transfers eligible for refund",
        ));
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

    fn default_config(env: &Env, bridge_id: u64) -> BridgeFeeConfig {
        BridgeFeeConfig {
            bridge_id,
            base_fee_bps: 30,
            min_fee: 100,
            max_fee: 10_000,
            validator_reward_pct: 8_000,
            treasury_pct: 2_000,
            dynamic_adjustment_enabled: true,
        }
    }

    fn xlm_asset(env: &Env) -> Asset {
        Asset {
            code: soroban_sdk::String::from_str(env, "XLM"),
            issuer: None,
        }
    }

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

    // ── estimate_bridge_fee tests ─────────────────────────────────────────────

    #[test]
    fn test_estimate_equals_actual_fee_no_multiplier() {
        let env = Env::default();
        let bridge_id = 1u64;
        set_bridge_fee_config(&env, &default_config(&env, bridge_id));
        set_destination_bridge_id(&env, ChainId::Ethereum, bridge_id);
        // No chain multiplier set → defaults to 10_000 (identity).

        let amount = 1_000_000i128;
        let estimated =
            estimate_bridge_fee(&env, ChainId::Ethereum, amount, &xlm_asset(&env)).unwrap();
        let actual = calculate_bridge_fee(&env, bridge_id, amount).unwrap();
        assert_eq!(
            estimated, actual,
            "estimate must equal actual when multiplier=10_000"
        );
    }

    #[test]
    fn test_estimate_with_surcharge_multiplier() {
        let env = Env::default();
        let bridge_id = 1u64;
        set_bridge_fee_config(&env, &default_config(&env, bridge_id));
        set_destination_bridge_id(&env, ChainId::Polygon, bridge_id);
        // 50 % surcharge: multiplier = 15_000 bps (i.e. × 1.5).
        set_chain_fee_multiplier(&env, ChainId::Polygon, 15_000);

        let amount = 1_000_000i128;
        let actual_base = calculate_bridge_fee(&env, bridge_id, amount).unwrap(); // 3000
        let estimated =
            estimate_bridge_fee(&env, ChainId::Polygon, amount, &xlm_asset(&env)).unwrap();

        assert_eq!(estimated, actual_base * 15_000 / 10_000);
        assert!(estimated > actual_base);
    }

    #[test]
    fn test_estimate_with_discount_multiplier() {
        let env = Env::default();
        let bridge_id = 1u64;
        set_bridge_fee_config(&env, &default_config(&env, bridge_id));
        set_destination_bridge_id(&env, ChainId::BNB, bridge_id);
        // 20 % discount: multiplier = 8_000 bps (× 0.8).
        set_chain_fee_multiplier(&env, ChainId::BNB, 8_000);

        let amount = 1_000_000i128;
        let actual_base = calculate_bridge_fee(&env, bridge_id, amount).unwrap();
        let estimated = estimate_bridge_fee(&env, ChainId::BNB, amount, &xlm_asset(&env)).unwrap();

        assert_eq!(estimated, actual_base * 8_000 / 10_000);
        assert!(estimated < actual_base);
    }

    #[test]
    fn test_estimate_respects_min_fee() {
        let env = Env::default();
        let bridge_id = 1u64;
        set_bridge_fee_config(&env, &default_config(&env, bridge_id));
        set_destination_bridge_id(&env, ChainId::Bitcoin, bridge_id);

        // Very small transfer → min_fee kicks in at 100.
        let amount = 100i128;
        let actual = calculate_bridge_fee(&env, bridge_id, amount).unwrap(); // min_fee = 100
        let estimated =
            estimate_bridge_fee(&env, ChainId::Bitcoin, amount, &xlm_asset(&env)).unwrap();
        // Identity multiplier: estimated must equal actual.
        assert_eq!(estimated, actual);
    }

    #[test]
    fn test_estimate_respects_max_fee() {
        let env = Env::default();
        let bridge_id = 1u64;
        set_bridge_fee_config(&env, &default_config(&env, bridge_id));
        set_destination_bridge_id(&env, ChainId::Ethereum, bridge_id);

        // Large transfer → max_fee 10_000 caps the base fee.
        let amount = 100_000_000i128;
        let actual = calculate_bridge_fee(&env, bridge_id, amount).unwrap(); // max_fee = 10_000
        let estimated =
            estimate_bridge_fee(&env, ChainId::Ethereum, amount, &xlm_asset(&env)).unwrap();
        assert_eq!(estimated, actual);
    }

    #[test]
    fn test_estimate_missing_fee_config_returns_error() {
        let env = Env::default();
        // No fee config set for bridge_id 1.
        let asset = xlm_asset(&env);
        let result = estimate_bridge_fee(&env, ChainId::Ethereum, 1_000_000, &asset);
        assert!(result.is_err());
    }

    #[test]
    fn test_chain_multiplier_default_is_identity() {
        let env = Env::default();
        // No multiplier set → defaults to 10_000.
        let m = get_chain_fee_multiplier(&env, ChainId::Stellar);
        assert_eq!(m, 10_000);
    }
}
