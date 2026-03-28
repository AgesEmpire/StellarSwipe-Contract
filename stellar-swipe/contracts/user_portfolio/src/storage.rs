use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Initialized,
    Admin,
    Oracle,
    NextPositionId,
    Position(u64),
    UserPositions(Address),
}
