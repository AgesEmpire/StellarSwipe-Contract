feature/copy-trade-balance-check
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

use soroban_sdk::contracterror;
 main

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    NotInitialized = 1,
 feature/copy-trade-balance-check
    PositionLimitReached = 2,
    InsufficientBalance = 3,
    InvalidAmount = 4,

feature/position-limit-copy-trade
    PositionLimitReached = 2,

    InvalidAmount = 2,
    SlippageExceeded = 3,
main
 main
}
