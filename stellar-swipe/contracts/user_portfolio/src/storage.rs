use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Initialized,
    Admin,
    Oracle,
    OracleAssetPair,
    NextPositionId,
    Position(u64),
    UserPositions(Address),
    /// Registered TradeExecutor contract allowed to call `close_position_keeper`.
    TradeExecutor,
    /// Per-user KYC verification flag (bool). No PII stored — boolean only.
    KycVerified(Address),
    /// Global KYC-required mode (bool). When true, only KYC-verified users can trade.
    KycRequiredMode,
    /// Per-user geographic restriction flag (bool). Restricted users cannot trade.
    Restricted(Address),
}
