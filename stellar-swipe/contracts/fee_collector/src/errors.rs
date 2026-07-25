use soroban_sdk::contracterror;

#[contracterror]
#[derive(Debug, PartialEq)]
#[repr(u32)]
pub enum ContractError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InsufficientTreasuryBalance = 5,
    WithdrawalNotQueued = 6,
    TimelockNotElapsed = 7,
    ArithmeticOverflow = 8,
    FeeRateTooHigh = 9,
    FeeRateTooLow = 10,
    OracleNotConfigured = 11,
    OracleConversionFailed = 12,
    FeeRoundedToZero = 13,
    BurnRateTooHigh = 14,
    DivisionByZero = 15,
    InvalidFeeConfiguration = 16,
    NetworkConditionInvalid = 17,
    FailedCollectionNotFound = 18,
    RetryLimitExceeded = 19,
    IterationLimitExceeded = 20,
    WaterfallNotConfigured = 21,
    PreferredTokenInsufficient = 22,
    PayoutCurrencyUnchanged = 23,
    InvalidMultiplierBounds = 24,
    SelfReferralNotAllowed = 25,
    ReferralAlreadyRegistered = 26,
    /// Issue #811: `upgrade()` was called with a version that is not
    /// strictly greater than the currently stored contract version.
    IncompatibleContractVersion = 27,
    /// Issue #813: caller is neither the admin nor on the authorized-caller
    /// allowlist for this privileged, non-user-scoped entry point.
    UnauthorizedCaller = 28,
    /// Issue #821: a fund-moving entry point was called while the shared
    /// circuit breaker has the contract paused.
    ContractPaused = 29,
}
