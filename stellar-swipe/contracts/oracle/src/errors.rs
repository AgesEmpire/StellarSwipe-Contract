//! Oracle error types

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OracleError {
    /// No price has been recorded for the requested asset.
    PriceNotFound = 1,
    /// No route of intermediate assets connects the source and target asset.
    NoConversionPath = 2,
    /// Conversion path supplied by the caller is empty or references an unknown asset.
    InvalidPath = 3,
    /// Multiplying/chaining prices along the conversion path overflowed.
    ConversionOverflow = 4,
    /// Caller is not authorized to perform this action.
    Unauthorized = 5,
    /// Asset is not recognized/registered with this oracle.
    InvalidAsset = 6,
    /// Latest price for this asset is older than the allowed staleness window.
    StalePrice = 7,
    /// No oracle source is registered for the requested feed.
    OracleNotFound = 8,
    /// Submitted price is zero, negative, or otherwise out of valid range.
    InvalidPrice = 9,
    /// An oracle source with this identifier is already registered.
    OracleAlreadyExists = 10,
    /// Fewer oracle sources are registered than the minimum required for consensus.
    InsufficientOracles = 11,
    /// Oracle's reputation score is below the minimum required to submit prices.
    LowReputation = 12,
    /// Not enough historical price points are recorded to compute the requested statistic.
    InsufficientHistoricalData = 13,
    /// Aggregated price failed reliability checks (e.g. too much source disagreement).
    UnreliablePrice = 14,
    /// No trading path was found between the requested assets.
    NoPathFound = 15,
    /// Executed price would deviate from the reference price by more than the allowed slippage.
    SlippageExceeded = 16,
    /// Order book for this asset pair has no resting orders.
    EmptyOrderBook = 17,
    /// Bid/ask spread is wider than the configured maximum.
    WideSpreadDetected = 18,
    /// Available liquidity is insufficient to fill the requested amount.
    InsufficientLiquidity = 19,
    /// Arithmetic operation overflowed.
    Overflow = 20,
    /// Circuit breaker has tripped and is blocking further price updates/trades.
    CircuitBreakerTripped = 21,
    /// Trade was blocked because the underlying price is stale.
    PriceStaleTradeBlocked = 22,
    /// No pending admin-transfer request exists to accept.
    PendingAdminNotFound = 23,
    /// Pending admin-transfer request has expired and must be re-initiated.
    PendingAdminExpired = 24,
    /// Fewer price sources contributed than the minimum required for this computation.
    InsufficientSources = 25,
    /// A single-update price deviation exceeded the configured maximum percentage.
    PriceDeviationBreakerTripped = 26,
    /// Issue #811: `upgrade()` was called with a version that is not
    /// strictly greater than the currently stored contract version.
    IncompatibleContractVersion = 27,
    /// Issue #864: a submitted quote's confidence is below the configured minimum.
    LowConfidence = 28,
    /// Issue #864: cross-source deviation exceeded the configured hard reject threshold.
    DeviationRejected = 29,
}

impl OracleError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            OracleError::PriceNotFound => "no price has been recorded for the requested asset",
            OracleError::NoConversionPath => {
                "no route of intermediate assets connects the source and target asset"
            }
            OracleError::InvalidPath => {
                "conversion path is empty or references an unknown asset"
            }
            OracleError::ConversionOverflow => {
                "multiplying/chaining prices along the conversion path overflowed"
            }
            OracleError::Unauthorized => "caller is not authorized to perform this action",
            OracleError::InvalidAsset => "asset is not recognized/registered with this oracle",
            OracleError::StalePrice => {
                "latest price for this asset is older than the allowed staleness window"
            }
            OracleError::OracleNotFound => "no oracle source is registered for the requested feed",
            OracleError::InvalidPrice => "submitted price is zero, negative, or out of range",
            OracleError::OracleAlreadyExists => {
                "an oracle source with this identifier is already registered"
            }
            OracleError::InsufficientOracles => {
                "fewer oracle sources are registered than required for consensus"
            }
            OracleError::LowReputation => {
                "oracle's reputation score is below the minimum required to submit prices"
            }
            OracleError::InsufficientHistoricalData => {
                "not enough historical price points to compute the requested statistic"
            }
            OracleError::UnreliablePrice => {
                "aggregated price failed reliability checks (excess source disagreement)"
            }
            OracleError::NoPathFound => "no trading path was found between the requested assets",
            OracleError::SlippageExceeded => {
                "executed price deviates from the reference price by more than allowed slippage"
            }
            OracleError::EmptyOrderBook => "order book for this asset pair has no resting orders",
            OracleError::WideSpreadDetected => "bid/ask spread is wider than the configured maximum",
            OracleError::InsufficientLiquidity => {
                "available liquidity is insufficient to fill the requested amount"
            }
            OracleError::Overflow => "arithmetic operation overflowed",
            OracleError::CircuitBreakerTripped => {
                "circuit breaker has tripped and is blocking further price updates/trades"
            }
            OracleError::PriceStaleTradeBlocked => "trade was blocked because the price is stale",
            OracleError::PendingAdminNotFound => {
                "no pending admin-transfer request exists to accept"
            }
            OracleError::PendingAdminExpired => {
                "pending admin-transfer request has expired and must be re-initiated"
            }
            OracleError::InsufficientSources => {
                "fewer price sources contributed than required for this computation"
            }
            OracleError::PriceDeviationBreakerTripped => {
                "a single-update price deviation exceeded the configured maximum percentage"
            }
            OracleError::IncompatibleContractVersion => {
                "upgrade() version is not strictly greater than the currently stored version"
            }
        }
    }
}
