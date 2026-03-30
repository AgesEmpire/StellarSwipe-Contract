use soroban_sdk::{contracttype, Address, Bytes, Env, Vec};
use stellar_swipe_common::AssetPair;

#[contracttype]
#[derive(Clone, Debug)]
pub struct OracleReputation {
    pub total_submissions: u32,
    pub accurate_submissions: u32,
    pub avg_deviation: i128,
    pub reputation_score: u32,
    pub weight: u32,
    pub last_slash: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceSubmission {
    pub oracle: Address,
    pub price: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceData {
    pub asset_pair: AssetPair,
    pub price: i128,
    pub timestamp: u64,
    pub source: Address,
    pub confidence: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    PriceMap(AssetPair),
    OracleStats,
    Oracles,
    PriceSubmissions,
    ConsensusPrice,
    PauseStates,
    OracleWeight(Address),
    PendingAdmin,
    PendingAdminExpiry,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ConsensusPriceData {
    pub price: i128,
    pub timestamp: u64,
    pub num_oracles: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ExternalPrice {
    pub asset_pair: AssetPair,
    pub price: i128,
    pub timestamp: u64,
    pub round_id: u64,
    pub signature: Bytes,
    pub oracle_address: Address,
}
