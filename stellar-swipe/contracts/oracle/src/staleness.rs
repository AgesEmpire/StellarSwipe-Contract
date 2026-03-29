use soroban_sdk::{contracttype, Env};
use stellar_swipe_common::AssetPair;

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
#[derive(Clone, Debug)]
pub struct PriceMetadata {
    pub last_update: u64,
    pub update_count_24h: u32,
    pub avg_update_interval: u64,
    pub staleness_level: StalenessLevel,
    pub is_paused: bool,
}

pub fn default_metadata() -> PriceMetadata {
    PriceMetadata {
        last_update: 0,
        update_count_24h: 0,
        avg_update_interval: 0,
        staleness_level: StalenessLevel::Critical,
        is_paused: false,
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
