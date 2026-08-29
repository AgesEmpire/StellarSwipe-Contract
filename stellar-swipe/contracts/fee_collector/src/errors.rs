use soroban_sdk::contracterror;

#[contracterror]
#[derive(Debug, PartialEq)]
#[repr(u32)]
pub enum ContractError {
    /// `initialize()` was called on a contract that already has an admin set.
    AlreadyInitialized = 1,
    /// Contract function was called before `initialize()` set up the admin/config.
    NotInitialized = 2,
    /// Caller is not authorized to perform this action.
    Unauthorized = 3,
    /// Amount is zero, negative, or otherwise outside allowed bounds.
    InvalidAmount = 4,
    /// Treasury balance is lower than the amount requested to withdraw.
    InsufficientTreasuryBalance = 5,
    /// No queued withdrawal request exists for this caller/id.
    WithdrawalNotQueued = 6,
    /// Time-lock period for this queued withdrawal has not yet elapsed.
    TimelockNotElapsed = 7,
    /// Arithmetic operation overflowed.
    ArithmeticOverflow = 8,
    /// Requested fee rate exceeds the configured maximum.
    FeeRateTooHigh = 9,
    /// Requested fee rate is below the configured minimum.
    FeeRateTooLow = 10,
    /// No price oracle has been configured for fee/currency conversion.
    OracleNotConfigured = 11,
    /// Oracle price conversion failed (stale, unavailable, or invalid price).
    OracleConversionFailed = 12,
    /// Computed fee rounded down to zero and was skipped.
    FeeRoundedToZero = 13,
    /// Requested burn rate exceeds the configured maximum.
    BurnRateTooHigh = 14,
    /// Attempted division by zero in a fee/rate calculation.
    DivisionByZero = 15,
    /// Fee configuration is invalid (e.g. rates don't sum correctly or are out of range).
    InvalidFeeConfiguration = 16,
    /// Network condition parameter used for dynamic fee adjustment is invalid.
    NetworkConditionInvalid = 17,
    /// No failed collection record exists for the given id.
    FailedCollectionNotFound = 18,
    /// Retry limit for a failed collection has already been reached.
    RetryLimitExceeded = 19,
    /// Operation would exceed the maximum allowed iteration/batch count.
    IterationLimitExceeded = 20,
    /// Fee waterfall distribution has not been configured.
    WaterfallNotConfigured = 21,
    /// Preferred payout token balance is insufficient for this payout.
    PreferredTokenInsufficient = 22,
    /// Requested payout currency is the same as the currently configured one.
    PayoutCurrencyUnchanged = 23,
    /// Multiplier value is outside the allowed bounds.
    InvalidMultiplierBounds = 24,
    /// A user cannot refer themselves.
    SelfReferralNotAllowed = 25,
    /// This referral relationship has already been registered.
    ReferralAlreadyRegistered = 26,
    /// Issue #811: `upgrade()` was called with a version that is not
    /// strictly greater than the currently stored contract version.
    IncompatibleContractVersion = 27,
    /// Issue #813: caller is neither the admin nor on the authorized-caller
    /// allowlist for this privileged, non-user-scoped entry point.
    UnauthorizedCaller = 28,
    /// Issue #814: no snapshot data exists for the requested ledger sequence.
    SnapshotNotFound = 29,
    /// Token metadata is missing, invalid, or ambiguous.
    InvalidTokenMetadata = 30,
    /// Requested payout exceeds the configured maximum payout cap per claim (#960).
    PayoutExceedsCap = 31,
    /// Insurance balance is insufficient for requested payout.
    InsufficientInsuranceBalance = 32,
    /// Claim ID has already been paid out (#960).
    ClaimAlreadyProcessed = 33,
    /// Contract is currently paused (#561).
    ContractPaused = 34,
}

impl ContractError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            ContractError::AlreadyInitialized => {
                "contract has already been initialized with an admin"
            }
            ContractError::NotInitialized => {
                "contract has not been initialized yet; call initialize() first"
            }
            ContractError::Unauthorized => "caller is not authorized to perform this action",
            ContractError::InvalidAmount => "amount is zero, negative, or outside allowed bounds",
            ContractError::InsufficientTreasuryBalance => {
                "treasury balance is lower than the amount requested to withdraw"
            }
            ContractError::WithdrawalNotQueued => {
                "no queued withdrawal request exists for this caller/id"
            }
            ContractError::TimelockNotElapsed => {
                "time-lock period for this queued withdrawal has not yet elapsed"
            }
            ContractError::ArithmeticOverflow => "arithmetic operation overflowed",
            ContractError::FeeRateTooHigh => "requested fee rate exceeds the configured maximum",
            ContractError::FeeRateTooLow => "requested fee rate is below the configured minimum",
            ContractError::OracleNotConfigured => {
                "no price oracle has been configured for fee/currency conversion"
            }
            ContractError::OracleConversionFailed => {
                "oracle price conversion failed (stale, unavailable, or invalid price)"
            }
            ContractError::FeeRoundedToZero => "computed fee rounded down to zero and was skipped",
            ContractError::BurnRateTooHigh => "requested burn rate exceeds the configured maximum",
            ContractError::DivisionByZero => "attempted division by zero in a fee/rate calculation",
            ContractError::InvalidFeeConfiguration => {
                "fee configuration is invalid (rates don't sum correctly or are out of range)"
            }
            ContractError::NetworkConditionInvalid => {
                "network condition parameter for dynamic fee adjustment is invalid"
            }
            ContractError::FailedCollectionNotFound => {
                "no failed collection record exists for the given id"
            }
            ContractError::RetryLimitExceeded => {
                "retry limit for this failed collection has already been reached"
            }
            ContractError::IterationLimitExceeded => {
                "operation would exceed the maximum allowed iteration/batch count"
            }
            ContractError::WaterfallNotConfigured => {
                "fee waterfall distribution has not been configured"
            }
            ContractError::PreferredTokenInsufficient => {
                "preferred payout token balance is insufficient for this payout"
            }
            ContractError::PayoutCurrencyUnchanged => {
                "requested payout currency is the same as the currently configured one"
            }
            ContractError::InvalidMultiplierBounds => "multiplier value is outside allowed bounds",
            ContractError::SelfReferralNotAllowed => "a user cannot refer themselves",
            ContractError::ReferralAlreadyRegistered => {
                "this referral relationship has already been registered"
            }
            ContractError::IncompatibleContractVersion => {
                "upgrade() version is not strictly greater than the currently stored version"
            }
            ContractError::UnauthorizedCaller => {
                "caller is neither the admin nor on the authorized-caller allowlist"
            }
            ContractError::SnapshotNotFound => {
                "no fee snapshot exists for the requested ledger sequence"
            }
            ContractError::InvalidTokenMetadata => {
                "token metadata is missing, invalid, or ambiguous"
            }
            ContractError::PayoutExceedsCap => {
                "requested payout exceeds the configured maximum payout cap per claim"
            }
            ContractError::InsufficientInsuranceBalance => {
                "insurance balance is insufficient for requested payout"
            }
            ContractError::ClaimAlreadyProcessed => "claim ID has already been paid out",
            ContractError::ContractPaused => "contract is currently paused",
        }
    }
}
