//! Oracle error types

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OracleError {
    PriceNotFound = 1,
    NoConversionPath = 2,
    InvalidPath = 3,
    ConversionOverflow = 4,
    Unauthorized = 5,
    InvalidAsset = 6,
    StalePrice = 7,
    OracleNotFound = 8,
    InvalidPrice = 9,
    OracleAlreadyExists = 10,
    InsufficientOracles = 11,
    LowReputation = 12,
    InsufficientHistoricalData = 13,
    UnreliablePrice = 14,
    NoPathFound = 15,
    SlippageExceeded = 16,
    EmptyOrderBook = 17,
    WideSpreadDetected = 18,
    InsufficientLiquidity = 19,
    Overflow = 20,
    CircuitBreakerTripped = 21,
    PriceStaleTradeBlocked = 22,
    PendingAdminNotFound = 23,
    PendingAdminExpired = 24,
}
