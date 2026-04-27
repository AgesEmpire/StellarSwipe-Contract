use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Initialized,
    Admin,
    Oracle,
    NextPositionId,
    Position(u64),
    /// V1: mixed open+closed list (preserved for migration reads).
    UserPositions(Address),
    /// V2: open positions only.
    UserOpenPositions(Address),
    /// V2: closed positions only.
    UserClosedPositions(Address),
    /// Migration: flag set once a user has been migrated.
    MigratedUser(Address),
    /// Migration: queue of users pending migration.
    MigrationQueue,
}
