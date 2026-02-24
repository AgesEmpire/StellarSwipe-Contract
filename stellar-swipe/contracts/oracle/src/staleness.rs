// contracts/oracle/src/staleness.rs
use soroban_sdk::{contracttype, Env, Address, symbol_short, Symbol};
use common::AssetPair;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StalenessLevel {
    Fresh,    // < 2m
    Aging,    // 2-5m
    Stale,    // 5-15m
    Critical, // > 15m
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceMetadata {
    pub last_update: u64,
    pub update_count_24h: u32,
    pub avg_update_interval: u64,
    pub staleness_level: StalenessLevel,
    pub is_paused: bool,
}

pub fn check_staleness(pair: AssetPair, current_time: u64) -> StalenessLevel {
    let metadata = get_price_metadata(pair);
    let age = current_time.saturating_sub(metadata.last_update);

    // thresholds can be pulled from a PairConfig
    match age {
        0..=120 => StalenessLevel::Fresh,
        121..=300 => StalenessLevel::Aging,
        301..=900 => StalenessLevel::Stale,
        _ => StalenessLevel::Critical,
    }
}