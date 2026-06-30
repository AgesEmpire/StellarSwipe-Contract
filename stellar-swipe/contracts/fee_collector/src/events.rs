use crate::storage::WaterfallTierResult;
use shared::errors::{ErrorCategory, RecoveryStrategy};
use soroban_sdk::{contractevent, Address, Env, String, Symbol, Vec};

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
