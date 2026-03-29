use soroban_sdk::{contractevent, Address};

#[contractevent]
pub struct WithdrawalQueued {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub available_at: u64,
}

#[contractevent]
pub struct TreasuryWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub remaining_balance: i128,
}
