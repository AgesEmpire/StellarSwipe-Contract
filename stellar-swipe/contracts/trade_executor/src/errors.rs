use soroban_sdk::{contracterror, contracttype};

/// Populated when [`crate::ContractError::InsufficientBalance`] is returned from
/// [`crate::TradeExecutorContract::execute_copy_trade`]; query via
/// [`crate::TradeExecutorContract::get_insufficient_balance_detail`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsufficientBalanceDetail {
    pub required: i128,
    pub available: i128,
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
}
