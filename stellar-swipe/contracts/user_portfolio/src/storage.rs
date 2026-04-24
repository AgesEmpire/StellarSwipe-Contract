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
}
