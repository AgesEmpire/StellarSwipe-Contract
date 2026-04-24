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
}
