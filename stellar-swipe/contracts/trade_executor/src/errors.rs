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
#[cfg_attr(test, derive(soroban_sdk::testutils::arbitrary::Arbitrary))]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    /// Contract function was called before `initialize()` set up the admin/config.
    NotInitialized = 1,
    /// Caller already holds the maximum number of allowed open positions.
    PositionLimitReached = 2,
    /// Caller's balance is lower than the amount required for this trade.
    InsufficientBalance = 3,
    /// Amount is zero, negative, or otherwise outside allowed bounds.
    InvalidAmount = 4,
    /// Re-entrant call into a state-mutating function was detected and rejected.
    ReentrancyDetected = 5,
    /// Caller is not authorized to perform this action.
    Unauthorized = 6,
    /// No trade exists for the given trade id.
    TradeNotFound = 7,
    /// Executed price would deviate from the reference price by more than the allowed slippage.
    SlippageExceeded = 8,
    /// Requested position size exceeds the maximum allowed percentage of the portfolio.
    PositionPctTooHigh = 9,
    /// Oracle price for this asset is older than the allowed staleness window.
    OraclePriceStale = 10,
    /// No oracle price is currently available for this asset.
    OracleUnavailable = 11,
    /// Trade would push the rolling daily volume past the configured limit.
    DailyVolumeLimitExceeded = 12,
    /// Oracle address is not on the admin-managed whitelist.
    OracleNotWhitelisted = 13,
    /// Cannot remove the last remaining whitelisted oracle (would leave none configured).
    CannotRemoveLastOracle = 14,
    /// Trade would push open interest past the configured maximum.
    OpenInterestLimitReached = 15,
    /// No DCA (dollar-cost-averaging) plan exists for the given id.
    DCAPlanNotFound = 16,
    /// A DCA plan with this id/configuration already exists.
    DCAPlanAlreadyExists = 17,
    /// The trade signal referenced by this action has expired.
    SignalExpired = 18,
    /// The next scheduled interval for this recurring action has not been reached yet.
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
    /// Trade executed but the confirmation depth has not yet been reached.
    ConfirmationDepthNotReached = 30,
    /// The user already holds the maximum number of concurrent open positions.
    TooManyOpenPositions = 31,
    /// The submitted nonce has already been committed (replay of a previously
    /// executed or currently in-flight trade). See the replay-protection audit
    /// (Issue: nonce replay attack prevention).
    NonceAlreadyUsed = 32,
    /// The trade's `expiry_ts` has passed; the replay-protection window for this
    /// nonce/tx_hash has closed. See the replay-protection audit
    /// (Issue: nonce replay attack prevention).
    TradeExpired = 33,
    /// The contract is paused (governance-driven emergency pause). See Issue #865.
    ContractPaused = 34,
}

impl ContractError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            ContractError::NotInitialized => {
                "contract has not been initialized yet; call initialize() first"
            }
            ContractError::PositionLimitReached => {
                "caller already holds the maximum number of allowed open positions"
            }
            ContractError::InsufficientBalance => {
                "caller's balance is lower than the amount required for this trade"
            }
            ContractError::InvalidAmount => "amount is zero, negative, or outside allowed bounds",
            ContractError::ReentrancyDetected => "re-entrant call was detected and rejected",
            ContractError::Unauthorized => "caller is not authorized to perform this action",
            ContractError::TradeNotFound => "no trade exists for the given trade id",
            ContractError::SlippageExceeded => {
                "executed price deviates from the reference price by more than allowed slippage"
            }
            ContractError::PositionPctTooHigh => {
                "requested position size exceeds the maximum allowed portfolio percentage"
            }
            ContractError::OraclePriceStale => {
                "oracle price for this asset is older than the allowed staleness window"
            }
            ContractError::OracleUnavailable => "no oracle price is currently available",
            ContractError::DailyVolumeLimitExceeded => {
                "trade would exceed the configured rolling daily volume limit"
            }
            ContractError::OracleNotWhitelisted => {
                "oracle address is not on the admin-managed whitelist"
            }
            ContractError::CannotRemoveLastOracle => {
                "cannot remove the last remaining whitelisted oracle"
            }
            ContractError::OpenInterestLimitReached => {
                "trade would push open interest past the configured maximum"
            }
            ContractError::DCAPlanNotFound => "no DCA plan exists for the given id",
            ContractError::DCAPlanAlreadyExists => {
                "a DCA plan with this id/configuration already exists"
            }
            ContractError::SignalExpired => {
                "the trade signal referenced by this action has expired"
            }
            ContractError::IntervalNotDue => {
                "the next scheduled interval for this recurring action has not been reached yet"
            }
            ContractError::NetworkCongestion => {
                "network is congested; check retry_after_ledger via get_network_error_detail"
            }
            ContractError::InsufficientLiquidity => {
                "SDEX pair has zero or insufficient liquidity for this swap"
            }
            ContractError::CircuitBreakerActive => {
                "circuit breaker is active and blocking this operation"
            }
            ContractError::FeatureDisabled => {
                "requested feature is administratively disabled via the feature flag registry"
            }
            ContractError::ReplayDetected => {
                "replayed transaction detected (nonce mismatch, duplicate hash, or expired)"
            }
            ContractError::BelowMinimumTradeSize => {
                "trade amount is below the configured per-asset minimum"
            }
            ContractError::GracePeriodExpired => {
                "grace period to cancel this queued trade has already elapsed"
            }
            ContractError::QueuedTradeNotFound => "the queued trade was not found",
            ContractError::NotTradeOwner => "caller is not the owner of this trade",
            ContractError::MaxRetriesExceeded => {
                "queued trade exceeded the maximum retry limit and moved to the dead-letter queue"
            }
            ContractError::ConfirmationDepthNotReached => {
                "trade executed but the confirmation depth has not yet been reached"
            }
            ContractError::TooManyOpenPositions => {
                "caller already holds the maximum number of concurrent open positions"
            }
            ContractError::NonceAlreadyUsed => {
                "submitted nonce has already been committed by a previous trade"
            }
            ContractError::TradeExpired => {
                "trade's expiry_ts has passed; the replay-protection window has closed"
            }
            ContractError::ContractPaused => {
                "contract is paused (governance-driven emergency pause)"
            }
        }
    }
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
