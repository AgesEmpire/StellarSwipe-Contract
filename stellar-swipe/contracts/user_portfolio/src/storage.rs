use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Initialized,
    Admin,
    Oracle,
    /// Contract address allowed to call `close_position` on behalf of a user
    /// (e.g. TradeExecutorContract for stop-loss / cancel flows).
    AuthorizedExecutor,
    NextPositionId,
    Position(u64),
    UserPositions(Address),
}
