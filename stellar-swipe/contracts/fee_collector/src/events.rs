use crate::storage::WaterfallTierResult;
use shared::errors::{ErrorCategory, RecoveryStrategy};
use soroban_sdk::{contractevent, Address, Env, String, Symbol, Vec};

#[contractevent]
pub struct SnapshotRecorded {
    pub ledger: u64,
    pub timestamp: u64,
    pub total_amount: i128,
    pub entry_count: u32,
}

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

#[contractevent]
pub struct FeesBurned {
    pub amount: i128,
    pub token: Address,
}

#[contractevent]
pub struct NetworkConditionUpdated {
    pub score_bps: u32,
    pub note: String,
    pub updated_at: u64,
}

#[contractevent]
pub struct ErrorReported {
    pub category: ErrorCategory,
    pub strategy: RecoveryStrategy,
    pub message: String,
    pub timestamp: u64,
}

#[contractevent]
pub struct RetryAttempted {
    pub id: String,
    pub retry_count: u32,
    pub successful: bool,
    pub timestamp: u64,
}

/// Emitted when a user's first trade fee is waived (Issue #428).
#[contractevent]
pub struct FirstTradeFeeWaived {
    pub user: Address,
}

#[contractevent]
pub struct ReferralRegistered {
    pub referrer: Address,
    pub referee: Address,
}

#[contractevent]
pub struct ReferralFeeShareUpdated {
    pub old_bps: u32,
    pub new_bps: u32,
    pub updated_by: Address,
}

#[contractevent]
pub struct ReferralFeePaid {
    pub referrer: Address,
    pub referee: Address,
    pub token: Address,
    pub amount: i128,
}

// ── Emit helpers ──────────────────────────────────────────────────────────────

pub struct EvtWithdrawalQueued {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub available_at: u64,
}

pub struct EvtTreasuryWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub remaining_balance: i128,
}

pub struct EvtFeeRateUpdated {
    pub old_rate: u32,
    pub new_rate: u32,
    pub updated_by: Address,
}

pub struct EvtNetworkConditionUpdated {
    pub score_bps: u32,
    pub note: String,
    pub updated_at: u64,
}

pub struct EvtErrorReported {
    pub category: ErrorCategory,
    pub strategy: RecoveryStrategy,
    pub message: String,
    pub timestamp: u64,
}

pub struct EvtRetryAttempted {
    pub id: String,
    pub retry_count: u32,
    pub successful: bool,
    pub timestamp: u64,
}

pub struct EvtFeeCollected {
    pub trader: Address,
    pub token: Address,
    pub trade_amount: i128,
    pub fee_amount: i128,
    pub fee_rate_bps: u32,
}

pub struct EvtFeesClaimed {
    pub provider: Address,
    pub token: Address,
    pub amount: i128,
}

pub fn emit_withdrawal_queued(env: &Env, evt: EvtWithdrawalQueued) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "withdrawal_queued"),
        ),
        (evt.recipient, evt.token, evt.amount, evt.available_at),
    );
}

pub fn emit_treasury_withdrawal(env: &Env, evt: EvtTreasuryWithdrawal) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "treasury_withdrawal"),
        ),
        (evt.recipient, evt.token, evt.amount, evt.remaining_balance),
    );
}

pub fn emit_fee_rate_updated(env: &Env, evt: EvtFeeRateUpdated) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "fee_rate_updated"),
        ),
        (evt.old_rate, evt.new_rate, evt.updated_by),
    );
}

pub fn emit_network_condition_updated(env: &Env, evt: EvtNetworkConditionUpdated) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "network_condition_updated"),
        ),
        (evt.score_bps, evt.note, evt.updated_at),
    );
}

pub fn emit_error_reported(env: &Env, evt: EvtErrorReported) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "error_reported"),
        ),
        (evt.category, evt.strategy, evt.message, evt.timestamp),
    );
}

pub fn emit_retry_attempted(env: &Env, evt: EvtRetryAttempted) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "retry_attempted"),
        ),
        (evt.id, evt.retry_count, evt.successful, evt.timestamp),
    );
}

pub fn emit_fee_collected(env: &Env, evt: EvtFeeCollected) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "fee_collected"),
        ),
        (
            evt.trader,
            evt.token,
            evt.trade_amount,
            evt.fee_amount,
            evt.fee_rate_bps,
        ),
    );
}

pub fn emit_fees_claimed(env: &Env, evt: EvtFeesClaimed) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "fees_claimed"),
        ),
        (evt.provider, evt.token, evt.amount),
    );
}

pub fn emit_first_trade_fee_waived(env: &Env, user: &Address) {
    FirstTradeFeeWaived { user: user.clone() }.publish(env);
}

// ── Issue #442: Revenue Share Distributed event ─────────────────────

/// Emitted when a revenue share snapshot is taken and distributed.
pub fn emit_revenue_share_distributed(
    env: &Env,
    token: &Address,
    total_amount: i128,
    snapshot_ledger: u64,
) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "revenue_share_distributed"),
        ),
        (token.clone(), total_amount, snapshot_ledger),
    );
}

// ── #690: Waterfall Distribution event ──────────────────────────────────────

/// Emitted after a waterfall distribution run, listing every tier's allocation.
pub fn emit_waterfall_distribution(
    env: &Env,
    token: &Address,
    total_distributed: i128,
    tier_results: &Vec<WaterfallTierResult>,
) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "waterfall_distribution"),
        ),
        (token.clone(), total_distributed, tier_results.clone()),
    );
}

// ── #691: Payout Currency Set event ─────────────────────────────────────────

pub fn emit_payout_currency_set(env: &Env, provider: &Address, preferred_token: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "payout_currency_set"),
        ),
        (provider.clone(), preferred_token.clone()),
    );
}

// ── #813: Authorized-caller allowlist events ────────────────────────────────

pub fn emit_caller_authorized(env: &Env, caller: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "caller_authorized"),
        ),
        caller.clone(),
    );
}

pub fn emit_caller_revoked(env: &Env, caller: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "caller_revoked"),
        ),
        caller.clone(),
    );
}

// ── Referral events ─────────────────────────────────────────────────────────

pub fn emit_referral_registered(env: &Env, referrer: &Address, referee: &Address) {
    ReferralRegistered {
        referrer: referrer.clone(),
        referee: referee.clone(),
    }
    .publish(env);
}

pub fn emit_referral_fee_share_updated(
    env: &Env,
    old_bps: u32,
    new_bps: u32,
    updated_by: &Address,
) {
    ReferralFeeShareUpdated {
        old_bps,
        new_bps,
        updated_by: updated_by.clone(),
    }
    .publish(env);
}

pub fn emit_referral_fee_paid(
    env: &Env,
    referrer: &Address,
    referee: &Address,
    token: &Address,
    amount: i128,
) {
    ReferralFeePaid {
        referrer: referrer.clone(),
        referee: referee.clone(),
        token: token.clone(),
        amount,
    }
    .publish(env);
}

// ── #665: Fee Forecast event ─────────────────────────────────────────────────

/// Emitted when a fee revenue forecast is computed (auto or manual trigger).
pub fn emit_fee_forecast(
    env: &Env,
    token: &Address,
    projected_amount: i128,
    basis_window_days: u64,
    current_epoch_day: u64,
) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "fee_forecast"),
        ),
        (
            token.clone(),
            projected_amount,
            basis_window_days,
            current_epoch_day,
        ),
    );
}

// ── #664: Volume Discount Config Updated event ───────────────────────────────

pub fn emit_volume_discount_config_updated(env: &Env, tier_count: u32) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "vol_discount_updated"),
        ),
        (tier_count,),
    );
}

pub fn emit_fees_claimed_converted(
    env: &Env,
    provider: &Address,
    source_token: &Address,
    preferred_token: &Address,
    source_amount: i128,
    preferred_amount: i128,
) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "fees_claimed_converted"),
        ),
        (
            provider.clone(),
            source_token.clone(),
            preferred_token.clone(),
            source_amount,
            preferred_amount,
        ),
    );
}

// ── Congestion-Based Dynamic Fees event ──────────────────────────────────────

#[contractevent]
pub struct EffectiveMultiplierChanged {
    pub old_multiplier_bps: u32,
    pub new_multiplier_bps: u32,
}

pub struct EvtEffectiveMultiplierChanged {
    pub old_multiplier_bps: u32,
    pub new_multiplier_bps: u32,
}

pub fn emit_effective_multiplier_changed(env: &Env, evt: EvtEffectiveMultiplierChanged) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "multiplier_changed"),
        ),
        (evt.old_multiplier_bps, evt.new_multiplier_bps),
    );
}

// ── Issue #814: Snapshot Recorded event ──────────────────────────────

pub struct EvtSnapshotRecorded {
    pub ledger: u64,
    pub timestamp: u64,
    pub total_amount: i128,
    pub entry_count: u32,
}

pub fn emit_snapshot_recorded(env: &Env, evt: EvtSnapshotRecorded) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "snapshot_recorded"),
        ),
        (evt.ledger, evt.timestamp, evt.total_amount, evt.entry_count),
    );
}

// ── Issue #960: Insurance Payout & Cap events ────────────────────────────────

#[contractevent]
pub struct InsurancePayout {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub claim_id: String,
    pub timestamp: u64,
}

#[contractevent]
pub struct InsurancePayoutCapUpdated {
    pub token: Address,
    pub old_cap: i128,
    pub new_cap: i128,
    pub updated_by: Address,
}

pub struct EvtInsurancePayout {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub claim_id: String,
    pub timestamp: u64,
}

pub fn emit_insurance_payout(env: &Env, evt: EvtInsurancePayout) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "insurance_payout"),
        ),
        (
            evt.recipient,
            evt.token,
            evt.amount,
            evt.claim_id,
            evt.timestamp,
        ),
    );
}

pub fn emit_insurance_payout_cap_updated(
    env: &Env,
    token: &Address,
    old_cap: i128,
    new_cap: i128,
    updated_by: &Address,
) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "ins_cap_updated"),
        ),
        (token.clone(), old_cap, new_cap, updated_by.clone()),
    );
}

// ── Issue #940: Rebate Cap Applied event (duplicate: also closes #947) ──────

/// Emitted when the per-epoch rebate cap is triggered and provider claims
/// are scaled down proportionally.
pub struct EvtRebateCapApplied {
    pub epoch: u64,
    pub requested: i128,
    pub distributed: i128,
}

pub fn emit_rebate_cap_applied(env: &Env, evt: EvtRebateCapApplied) {
    env.events().publish(
        (
            Symbol::new(env, "fee_collector"),
            Symbol::new(env, "rebate_cap_applied"),
        ),
        (evt.epoch, evt.requested, evt.distributed),
    );
}
