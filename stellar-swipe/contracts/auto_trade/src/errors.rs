use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AutoTradeError {
    InvalidAmount = 1,
    Unauthorized = 2,
    SignalNotFound = 3,
    SignalExpired = 4,
    InsufficientBalance = 5,
    InsufficientLiquidity = 6,
    DailyTradeLimitExceeded = 7,
    PositionLimitExceeded = 8,
    StopLossTriggered = 9,
    // Rate limit errors
    RateLimitPenalty = 10,
    BelowMinTransfer = 11,
    CooldownNotElapsed = 12,
    HourlyTransferLimitExceeded = 13,
    HourlyVolumeLimitExceeded = 14,
    DailyTransferLimitExceeded = 15,
    DailyVolumeLimitExceeded = 16,
    GlobalCapacityExceeded = 17,
}
