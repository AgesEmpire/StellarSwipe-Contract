//! Fee collector events — re-exported from the shared event schema.
//! Use the helpers in `shared::events` to emit standardized events.
pub use shared::events::{
    emit_fee_collected, emit_fee_rate_updated, emit_fees_claimed, emit_treasury_withdrawal,
    emit_withdrawal_queued, EvtFeeCollected, EvtFeeRateUpdated, EvtFeesClaimed,
    EvtTreasuryWithdrawal, EvtWithdrawalQueued,
};
