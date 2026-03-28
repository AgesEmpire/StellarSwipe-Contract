#[derive(Copy, Clone, Debug, Eq, PartialEq)]
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
    DcaStrategyNotFound = 10,
    DcaStrategyInactive = 11,
    DcaEndTimeReached = 12,
    MrStrategyNotFound = 13,
    MrInsufficientHistory = 14,
    MrLowVolatility = 15,
docs/contract-events-documentation
    TradingPaused = 16,
    StrategyNotFound = 17,
    PositionAlreadyExists = 18,
    RankingDisabled = 19,
    InvalidBasketSize = 20,
    InsufficientPriceHistory = 21,
    InvalidPriceData = 22,
    NonCointegratedBasket = 23,
    ActivePortfolioExists = 24,
    NoActivePortfolio = 25,
    NoTradeSignal = 26,
    InvalidStatArbConfig = 27,
    PairsStrategyNotFound = 28,
    PairsActivePositionExists = 29,
    PairsNoActivePosition = 30,
    InsufficientCorrelation = 31,
    PairNotCointegrated = 32,
    InvalidPairsConfig = 33,
    ArbitrageOpportunityExpired = 34,
    ArbitrageUnprofitable = 35,
    ArbTooLarge = 36,
    FrontRunningRisk = 37,
    InvalidInsuranceConfig = 38,
    InsuranceNotConfigured = 39,
    SelfReferral = 40,
    ReferralAlreadySet = 41,
    CircularReferral = 42,
    ReferralLimitExceeded = 43,
    InvalidTWAPDuration = 44,
    TWAPOrderNotFound = 45,
    NotTWAPOwner = 46,
    TWAPNotActive = 47,
    CorrelationLimitExceeded = 48,
    TooManyCorrelatedPositions = 49,
    ConditionalOrderNotFound = 50,
    ConditionalOrderNotPending = 51,
    ConditionalOrderNotTriggered = 52,
    InvalidConditionalConfig = 53,
    RateLimitPenalty = 54,
    BelowMinTransfer = 55,
    CooldownNotElapsed = 56,
    HourlyTransferLimitExceeded = 57,
    HourlyVolumeLimitExceeded = 58,
    DailyTransferLimitExceeded = 59,
    DailyVolumeLimitExceeded = 60,
    GlobalCapacityExceeded = 61,


feature/dca-strategy
    DcaStrategyNotFound = 10,
    DcaStrategyInactive = 11,
    DcaEndTimeReached = 12,
 main

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
    
    // Pairs Trading
    PairsStrategyNotFound = 22,
    PairsActivePositionExists = 23,
    PairsNoActivePosition = 24,
    InsufficientCorrelation = 25,
    PairNotCointegrated = 26,
    InvalidPairsConfig = 27,
 
 main
    // Arbitrage
    ArbitrageOpportunityExpired = 28,
    ArbitrageUnprofitable = 29,
    ArbTooLarge = 30,
    FrontRunningRisk = 31,

    // Insurance
    InvalidInsuranceConfig = 32,
    InsuranceNotConfigured = 33,

    // Referral
    SelfReferral = 34,
    ReferralAlreadySet = 35,
    CircularReferral = 36,
    ReferralLimitExceeded = 37,

 TWAP-Orders
    // TWAP
    InvalidTWAPDuration = 38,
    TWAPOrderNotFound = 39,
    NotTWAPOwner = 40,
    TWAPNotActive = 41,

Correlation-Based-Risk
    // Correlation
    CorrelationLimitExceeded = 42,
    TooManyCorrelatedPositions = 43,

    // Conditional Orders
    ConditionalOrderNotFound = 44,
    ConditionalOrderNotPending = 45,
    ConditionalOrderNotTriggered = 46,
    InvalidConditionalConfig = 47,

    // Oracle circuit breaker
    OracleUnavailable = 48,

    // Oracle whitelist
    LastOracleForPair = 49,

 main
 main
main
 main
 main
}
