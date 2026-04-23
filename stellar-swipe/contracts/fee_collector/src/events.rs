// All fee_collector event structs are defined in the shared common crate.
// Re-export them here for backward compatibility within this crate.
pub use stellar_swipe_common::{
    EvtFeeRateUpdated as FeeRateUpdated, EvtFeesClaimed as FeesClaimed,
    EvtTreasuryWithdrawal as TreasuryWithdrawal, EvtWithdrawalQueued as WithdrawalQueued,
};
