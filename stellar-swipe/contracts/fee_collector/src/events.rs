use soroban_sdk::{contractevent, Address};

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

#[contractevent]
pub struct FeesClaimed {
    pub provider: Address,
    pub token: Address,
    pub amount: i128,
}
