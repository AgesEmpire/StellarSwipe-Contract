use soroban_sdk::contracterror;

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
    TradingPaused = 10,

    StrategyNotFound = 10,
    PositionAlreadyExists = 11,
    InsufficientPriceHistory = 12,
    RankingDisabled = 13,

    InvalidBasketSize = 10,
    InsufficientPriceHistory = 11,
    InvalidPriceData = 12,
    NonCointegratedBasket = 13,
    ActivePortfolioExists = 14,
    NoActivePortfolio = 15,
    NoTradeSignal = 16,
    InvalidStatArbConfig = 17,
 
}
