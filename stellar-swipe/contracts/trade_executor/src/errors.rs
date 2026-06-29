use soroban_sdk::{contracterror, contracttype};

/// Populated when [`ContractError::InsufficientBalance`] is returned from
/// [`crate::TradeExecutorContract::execute_copy_trade`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsufficientBalanceDetail {
    pub required: i128,
    pub available: i128,
}

/// Populated when [`ContractError::NetworkCongestion`] is returned.
/// `retry_after_ledger` is the earliest ledger at which the caller should retry.
/// A value of `0` means the contract has no estimate — retry at caller's discretion.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkErrorDetail {
    /// Earliest ledger sequence the caller should retry at.
    pub retry_after_ledger: u32,
    /// Whether this error is transient (true) or permanent (false).
    /// Frontend should only offer a retry option when `is_transient == true`.
    pub is_transient: bool,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    NotInitialized = 1,
    PositionLimitReached = 2,
    InsufficientBalance = 3,
    InvalidAmount = 4,
    ReentrancyDetected = 5,
    Unauthorized = 6,
    TradeNotFound = 7,
    SlippageExceeded = 8,
    PositionPctTooHigh = 9,
    OraclePriceStale = 10,
    OracleUnavailable = 11,
    DailyVolumeLimitExceeded = 12,
    OracleNotWhitelisted = 13,
    CannotRemoveLastOracle = 14,
    OpenInterestLimitReached = 15,
    DCAPlanNotFound = 16,
    DCAPlanAlreadyExists = 17,
    SignalExpired = 18,
    IntervalNotDue = 19,
    /// Transient: the network is congested. Caller should read `NetworkErrorDetail`
    /// via [`crate::TradeExecutorContract::get_network_error_detail`] and retry
    /// after `retry_after_ledger`.
    NetworkCongestion = 20,
    /// The SDEX pair has zero or insufficient liquidity. Check `InsufficientLiquidityDetail`
    /// for available liquidity and required amount. Try again later or reduce trade size.
    InsufficientLiquidity = 21,
    CircuitBreakerActive = 22,
    /// The requested feature is administratively disabled via the feature flag registry.
    FeatureDisabled = 23,
    /// A replayed transaction was detected (nonce mismatch, duplicate hash, or expired).
    ReplayDetected = 24,
    /// Trade amount is below the configured per-asset minimum (dust-amount griefing guard).
    BelowMinimumTradeSize = 25,
    /// Attempt to cancel a queued trade after the grace period has elapsed.
    GracePeriodExpired = 26,
    /// The queued trade was not found.
    QueuedTradeNotFound = 27,
    /// The caller is not the trade owner.
    NotTradeOwner = 28,
    /// A queued trade exceeded the maximum retry limit and was moved to the dead-letter queue.
    MaxRetriesExceeded = 29,
}

/// Populated when [`ContractError::InsufficientLiquidity`] is returned.
/// `available_liquidity` is the best ask quantity available; `required_amount` is what was requested.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsufficientLiquidityDetail {
    /// Amount of liquidity available at the best ask (0 if order book is empty).
    pub available_liquidity: i128,
    /// Amount required for the swap.
    pub required_amount: i128,
}
