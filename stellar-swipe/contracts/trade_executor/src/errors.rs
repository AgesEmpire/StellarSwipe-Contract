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
    DCAPlanNotFound = 15,
    DCAPlanAlreadyExists = 16,
    SignalExpired = 17,
    IntervalNotDue = 18,
    /// Transient: the network is congested. Caller should read `NetworkErrorDetail`
    /// via [`crate::TradeExecutorContract::get_network_error_detail`] and retry
    /// after `retry_after_ledger`.
    NetworkCongestion = 19,
}
