use soroban_sdk::{contractevent, Address, Symbol};

#[contractevent]
pub struct WithdrawalQueued {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub available_at: u64,
}

#[contractevent]
pub struct FeeRateUpdated {
    pub old_rate: u32,
    pub new_rate: u32,
    pub updated_by: Address,
}

#[contractevent]
pub struct TreasuryWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub remaining_balance: i128,
}

/// Emitted whenever a deprecated entry point is invoked via a compatibility shim.
#[contractevent]
pub struct DeprecatedCall {
    pub entry_point: Symbol,
    pub replacement: Symbol,
}
