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
    /// Cumulative closed trades per user (for FIRST_TRADE / TEN_TRADES).
    UserClosedTradeCount(Address),
    /// Consecutive closes with `realized_pnl > 0`.
    UserProfitStreak(Address),
    /// Earned badges (append-only; dedupe by `BadgeType`).
    UserBadges(Address),
    /// Leaderboard rank for this user (1 = best). 0 means unset / not in board.
    LeaderboardRank(Address),
    /// Max users who receive `EarlyAdopter` (set at init).
    EarlyAdopterCap,
    /// How many distinct users have opened at least one position (ordering for early adopter).
    TotalUsersFirstOpen,
    /// Closed position IDs in chronological close order (oldest close first) for pagination.
    UserClosedChronological(Address),
}
