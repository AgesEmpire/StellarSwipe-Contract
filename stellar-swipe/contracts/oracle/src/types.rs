use soroban_sdk::{contracttype, Address, Bytes};
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
    Guardian,
    PriceMap(AssetPair),
    OracleStats,
    Oracles,
    PriceSubmissions,
    ConsensusPrice,
    PauseStates,
    OracleWeight(Address),
    PendingAdmin,
    PendingAdminExpiry,
    MinSourceCount,
    /// Issue #864: minimum confidence (0-100) required for a submitted quote to be accepted.
    MinConfidence,
    /// Max percentage deviation allowed between consecutive price updates for an asset pair (basis points, 10000 = 100%).
    DeviationThreshold(AssetPair),
    /// Set to true when the single-update deviation breaker has tripped for an asset pair.
    DeviationBreakerTripped(AssetPair),
    /// Issue #865: central governance contract address authorized to call
    /// `apply_governance_pause`.
    GovernanceAddress,
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
    /// Decimal precision of the raw `price` value (e.g. 6 for USDC, 7 for XLM).
    pub decimals: u32,
}
