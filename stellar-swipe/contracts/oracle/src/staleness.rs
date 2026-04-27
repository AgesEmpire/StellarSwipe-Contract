use soroban_sdk::{contracttype, Env};
use stellar_swipe_common::AssetPair;

pub const MAX_PRICE_AGE_LEDGERS: u32 = 60;
pub const ORACLE_DEAD_THRESHOLD_LEDGERS: u32 = 1_440;

#[contracttype]
#[derive(Clone)]
enum StaleStorageKey {
    Meta(AssetPair),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StalenessLevel {
    Fresh,
    Aging,
    Stale,
    Critical,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleStatus {
    Healthy,
    Stale,
    Dead,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleHealth {
    pub is_healthy: bool,
    pub last_update_ledger: u32,
    pub ledgers_since_update: u32,
    pub status: OracleStatus,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceMetadata {
    pub last_update: u64,
    pub last_update_ledger: u32,
    pub update_count_24h: u32,
    pub avg_update_interval: u64,
    pub staleness_level: StalenessLevel,
    pub is_paused: bool,
    pub last_heartbeat_status: OracleStatus,
}

pub fn default_metadata() -> PriceMetadata {
    PriceMetadata {
        last_update: 0,
        last_update_ledger: 0,
        update_count_24h: 0,
        avg_update_interval: 0,
        staleness_level: StalenessLevel::Critical,
        is_paused: false,
        last_heartbeat_status: OracleStatus::Dead,
    }
}

fn load_metadata(env: &Env, pair: &AssetPair) -> PriceMetadata {
    env.storage()
        .instance()
        .get(&StaleStorageKey::Meta(pair.clone()))
        .unwrap_or_else(default_metadata)
}

pub fn get_metadata(env: &Env, pair: &AssetPair) -> PriceMetadata {
    load_metadata(env, pair)
}

pub fn set_metadata(env: &Env, pair: &AssetPair, metadata: PriceMetadata) {
    env.storage()
        .instance()
        .set(&StaleStorageKey::Meta(pair.clone()), &metadata);
}

pub fn check_staleness(env: &Env, pair: AssetPair) -> StalenessLevel {
    let metadata = load_metadata(env, &pair);
    let now = env.ledger().timestamp();
    let age = now.saturating_sub(metadata.last_update);

    match age {
        0..=120 => StalenessLevel::Fresh,
        121..=300 => StalenessLevel::Aging,
        301..=900 => StalenessLevel::Stale,
        _ => StalenessLevel::Critical,
    }
}

pub fn check_oracle_heartbeat(env: &Env, pair: &AssetPair) -> OracleHealth {
    let metadata = load_metadata(env, pair);
    let current_ledger = env.ledger().sequence();
    let ledgers_since_update = if metadata.last_update_ledger == 0 {
        u32::MAX
    } else {
        current_ledger.saturating_sub(metadata.last_update_ledger)
    };

    let status = if metadata.last_update_ledger == 0
        || ledgers_since_update > ORACLE_DEAD_THRESHOLD_LEDGERS
    {
        OracleStatus::Dead
    } else if ledgers_since_update > MAX_PRICE_AGE_LEDGERS {
        OracleStatus::Stale
    } else {
        OracleStatus::Healthy
    };

    OracleHealth {
        is_healthy: status == OracleStatus::Healthy,
        last_update_ledger: metadata.last_update_ledger,
        ledgers_since_update,
        status,
    }
}
