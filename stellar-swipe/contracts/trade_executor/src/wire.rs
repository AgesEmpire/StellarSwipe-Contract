//! XDR-compatible mirrors of `auto_trade` types used only for `Env::try_invoke_contract`.
//! Keep in sync with `contracts/auto_trade/src/lib.rs` and `contracts/auto_trade/src/errors.rs`.

use soroban_sdk::{contracterror, contracttype, Address};

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeStatus {
    Pending,
    PartiallyFilled,
    Filled,
    Failed,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trade {
    pub signal_id: u64,
    pub user: Address,
    pub requested_amount: i128,
    pub executed_amount: i128,
    pub executed_price: i128,
    pub timestamp: u64,
    pub status: TradeStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeResult {
    pub trade: Trade,
}

#[contracterror(export = false)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
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
    TradingPaused = 10,
    StrategyNotFound = 11,
    PositionAlreadyExists = 12,
    RankingDisabled = 13,
    InvalidBasketSize = 14,
    InsufficientPriceHistory = 15,
    InvalidPriceData = 16,
    NonCointegratedBasket = 17,
    ActivePortfolioExists = 18,
    NoActivePortfolio = 19,
    NoTradeSignal = 20,
    InvalidStatArbConfig = 21,
    PairsStrategyNotFound = 22,
    PairsActivePositionExists = 23,
    PairsNoActivePosition = 24,
    InsufficientCorrelation = 25,
    PairNotCointegrated = 26,
    InvalidPairsConfig = 27,
    ArbitrageOpportunityExpired = 28,
    ArbitrageUnprofitable = 29,
    ArbTooLarge = 30,
    FrontRunningRisk = 31,
    InvalidInsuranceConfig = 32,
    InsuranceNotConfigured = 33,
    SelfReferral = 34,
    ReferralAlreadySet = 35,
    CircularReferral = 36,
    ReferralLimitExceeded = 37,
    InvalidTWAPDuration = 38,
    TWAPOrderNotFound = 39,
    NotTWAPOwner = 40,
    TWAPNotActive = 41,
    CorrelationLimitExceeded = 42,
    TooManyCorrelatedPositions = 43,
    ConditionalOrderNotFound = 44,
    ConditionalOrderNotPending = 45,
    ConditionalOrderNotTriggered = 46,
    InvalidConditionalConfig = 47,
    DcaStrategyNotFound = 48,
    DcaStrategyInactive = 49,
    DcaEndTimeReached = 50,
    MrStrategyNotFound = 51,
    MrInsufficientHistory = 52,
    MrLowVolatility = 53,
    RateLimitPenalty = 54,
    BelowMinTransfer = 55,
    CooldownNotElapsed = 56,
    HourlyTransferLimitExceeded = 57,
    HourlyVolumeLimitExceeded = 58,
    DailyTransferLimitExceeded = 59,
    DailyVolumeLimitExceeded = 60,
    GlobalCapacityExceeded = 61,
    BridgePaused = 62,
    NotPaused = 63,
    RecoveryNotFound = 64,
    RecoveryIncomplete = 65,
}
