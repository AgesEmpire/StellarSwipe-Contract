//! Standardized event schema for all StellarSwipe contracts.
//!
//! ## Format
//! Every event uses a two-topic tuple:
//! ```text
//! topics: (contract_name: Symbol, event_name: Symbol)
//! body:   <EventStruct>  (a #[contracttype] struct)
//! ```
//!
//! This lets Horizon / indexers filter by contract and event name independently.
//!
//! ## Stability policy
//! Field names and types are **stable across contract versions**.
//! Adding new optional fields is allowed; removing or renaming fields is a
//! breaking change and requires a new event name.

use soroban_sdk::{contracttype, symbol_short, Address, Env, String, Symbol};

// ── Contract name symbols ─────────────────────────────────────────────────────
// Used as the first topic on every event.

pub fn contract_fee_collector(env: &Env) -> Symbol {
    Symbol::new(env, "fee_collector")
}
pub fn contract_trade_executor(env: &Env) -> Symbol {
    Symbol::new(env, "trade_executor")
}
pub fn contract_user_portfolio(env: &Env) -> Symbol {
    Symbol::new(env, "user_portfolio")
}
pub fn contract_signal_registry(env: &Env) -> Symbol {
    Symbol::new(env, "signal_registry")
}
pub fn contract_governance(env: &Env) -> Symbol {
    symbol_short!("governance")
}

// ═══════════════════════════════════════════════════════════════════════════════
// FeeCollector events
// ═══════════════════════════════════════════════════════════════════════════════

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtFeeCollected {
    pub trader: Address,
    pub token: Address,
    pub trade_amount: i128,
    pub fee_amount: i128,
    pub fee_rate_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtFeeRateUpdated {
    pub old_rate: u32,
    pub new_rate: u32,
    pub updated_by: Address,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtFeesClaimed {
    pub provider: Address,
    pub token: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtWithdrawalQueued {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub available_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtTreasuryWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub remaining_balance: i128,
}

pub fn emit_fee_collected(env: &Env, evt: EvtFeeCollected) {
    env.events().publish(
        (contract_fee_collector(env), Symbol::new(env, "fee_collected")),
        evt,
    );
}

pub fn emit_fee_rate_updated(env: &Env, evt: EvtFeeRateUpdated) {
    env.events().publish(
        (contract_fee_collector(env), Symbol::new(env, "fee_rate_updated")),
        evt,
    );
}

pub fn emit_fees_claimed(env: &Env, evt: EvtFeesClaimed) {
    env.events().publish(
        (contract_fee_collector(env), Symbol::new(env, "fees_claimed")),
        evt,
    );
}

pub fn emit_withdrawal_queued(env: &Env, evt: EvtWithdrawalQueued) {
    env.events().publish(
        (contract_fee_collector(env), Symbol::new(env, "withdrawal_queued")),
        evt,
    );
}

pub fn emit_treasury_withdrawal(env: &Env, evt: EvtTreasuryWithdrawal) {
    env.events().publish(
        (contract_fee_collector(env), Symbol::new(env, "treasury_withdrawal")),
        evt,
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TradeExecutor events
// ═══════════════════════════════════════════════════════════════════════════════

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtTradeCancelled {
    pub user: Address,
    pub trade_id: u64,
    pub exit_price: i128,
    pub realized_pnl: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtStopLossTriggered {
    pub user: Address,
    pub trade_id: u64,
    pub stop_loss_price: i128,
    pub current_price: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtTakeProfitTriggered {
    pub user: Address,
    pub trade_id: u64,
    pub take_profit_price: i128,
    pub current_price: i128,
}

pub fn emit_trade_cancelled(env: &Env, evt: EvtTradeCancelled) {
    env.events().publish(
        (contract_trade_executor(env), Symbol::new(env, "trade_cancelled")),
        evt,
    );
}

pub fn emit_stop_loss_triggered(env: &Env, evt: EvtStopLossTriggered) {
    env.events().publish(
        (contract_trade_executor(env), Symbol::new(env, "stop_loss_triggered")),
        evt,
    );
}

pub fn emit_take_profit_triggered(env: &Env, evt: EvtTakeProfitTriggered) {
    env.events().publish(
        (contract_trade_executor(env), Symbol::new(env, "take_profit_triggered")),
        evt,
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// UserPortfolio events
// ═══════════════════════════════════════════════════════════════════════════════

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtTradeShareable {
    pub user: Address,
    pub position_id: u64,
    pub asset_pair: u32,
    pub entry_price: i128,
    pub exit_price: i128,
    pub pnl_bps: i64,
    pub signal_provider: Address,
    pub signal_id: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtPositionClosedByKeeper {
    pub user: Address,
    pub position_id: u64,
    pub asset_pair: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtSubscriptionCreated {
    pub user: Address,
    pub provider: Address,
    pub expires_at: u64,
}

pub fn emit_trade_shareable(env: &Env, evt: EvtTradeShareable) {
    env.events().publish(
        (contract_user_portfolio(env), Symbol::new(env, "trade_shareable")),
        evt,
    );
}

pub fn emit_position_closed_by_keeper(env: &Env, evt: EvtPositionClosedByKeeper) {
    env.events().publish(
        (contract_user_portfolio(env), Symbol::new(env, "keeper_close")),
        evt,
    );
}

pub fn emit_subscription_created(env: &Env, evt: EvtSubscriptionCreated) {
    env.events().publish(
        (contract_user_portfolio(env), Symbol::new(env, "subscription_created")),
        evt,
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// SignalRegistry events
// ═══════════════════════════════════════════════════════════════════════════════

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtSignalAdopted {
    pub signal_id: u64,
    pub adopter: Address,
    pub new_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtSignalEdited {
    pub signal_id: u64,
    pub provider: Address,
    pub price: i128,
    pub rationale_hash: String,
    pub confidence: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtReputationUpdated {
    pub provider: Address,
    pub old_score: u32,
    pub new_score: u32,
}

pub fn emit_signal_adopted(env: &Env, evt: EvtSignalAdopted) {
    env.events().publish(
        (contract_signal_registry(env), Symbol::new(env, "signal_adopted")),
        evt,
    );
}

pub fn emit_signal_edited(env: &Env, evt: EvtSignalEdited) {
    env.events().publish(
        (contract_signal_registry(env), Symbol::new(env, "signal_edited")),
        evt,
    );
}

pub fn emit_reputation_updated(env: &Env, evt: EvtReputationUpdated) {
    env.events().publish(
        (contract_signal_registry(env), Symbol::new(env, "reputation_updated")),
        evt,
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Governance events
// ═══════════════════════════════════════════════════════════════════════════════

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtStakeChanged {
    pub holder: Address,
    pub amount: i128,
    pub is_stake: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtRewardClaimed {
    pub beneficiary: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EvtVestingReleased {
    pub beneficiary: Address,
    pub amount: i128,
}

pub fn emit_stake_changed(env: &Env, evt: EvtStakeChanged) {
    env.events().publish(
        (contract_governance(env), Symbol::new(env, "stake_changed")),
        evt,
    );
}

pub fn emit_reward_claimed(env: &Env, evt: EvtRewardClaimed) {
    env.events().publish(
        (contract_governance(env), Symbol::new(env, "reward_claimed")),
        evt,
    );
}

pub fn emit_vesting_released(env: &Env, evt: EvtVestingReleased) {
    env.events().publish(
        (contract_governance(env), Symbol::new(env, "vesting_released")),
        evt,
    );
}
