//! Standardised event schema for all StellarSwipe contracts.
//!
//! ## Format
//! Every event uses a two-topic tuple `(contract_name: Symbol, event_name: Symbol)` as
//! the topics argument, and a `#[contracttype]` struct as the data body.
//!
//! ```text
//! env.events().publish(
//!     (Symbol::new(env, CONTRACT_NAME), Symbol::new(env, EVENT_NAME)),
//!     EventStruct { ... },
//! );
//! ```
//!
//! Each struct exposes a `.publish(&env)` helper so call-sites are one line.
//!
//! ## Stability guarantee
//! Field names and types are **stable across contract versions**.  Adding new
//! optional fields is allowed; removing or renaming existing fields is a
//! breaking change and requires a major version bump.

use soroban_sdk::{contracttype, Address, Env, Symbol};

// ── fee_collector ─────────────────────────────────────────────────────────────

pub const CONTRACT_FEE_COLLECTOR: &str = "fee_collector";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtWithdrawalQueued {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub available_at: u64,
}

impl EvtWithdrawalQueued {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_FEE_COLLECTOR),
                Symbol::new(env, "withdrawal_queued"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtFeeRateUpdated {
    pub old_rate: u32,
    pub new_rate: u32,
    pub updated_by: Address,
}

impl EvtFeeRateUpdated {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_FEE_COLLECTOR),
                Symbol::new(env, "fee_rate_updated"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTreasuryWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub remaining_balance: i128,
}

impl EvtTreasuryWithdrawal {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_FEE_COLLECTOR),
                Symbol::new(env, "treasury_withdrawal"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtFeesClaimed {
    pub provider: Address,
    pub token: Address,
    pub amount: i128,
}

impl EvtFeesClaimed {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_FEE_COLLECTOR),
                Symbol::new(env, "fees_claimed"),
            ),
            self,
        );
    }
}

// ── user_portfolio ────────────────────────────────────────────────────────────

pub const CONTRACT_USER_PORTFOLIO: &str = "user_portfolio";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtBadgeAwarded {
    pub user: Address,
    pub badge_type: u32,
    pub awarded_at: u64,
}

impl EvtBadgeAwarded {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_USER_PORTFOLIO),
                Symbol::new(env, "badge_awarded"),
            ),
            self,
        );
    }
}

// ── trade_executor ────────────────────────────────────────────────────────────

pub const CONTRACT_TRADE_EXECUTOR: &str = "trade_executor";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtStopLossTriggered {
    pub user: Address,
    pub trade_id: u64,
    pub stop_loss_price: i128,
    pub current_price: i128,
}

impl EvtStopLossTriggered {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_TRADE_EXECUTOR),
                Symbol::new(env, "stop_loss_triggered"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTakeProfitTriggered {
    pub user: Address,
    pub trade_id: u64,
    pub take_profit_price: i128,
    pub current_price: i128,
}

impl EvtTakeProfitTriggered {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_TRADE_EXECUTOR),
                Symbol::new(env, "take_profit_triggered"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTradeCancelled {
    pub user: Address,
    pub trade_id: u64,
    pub exit_price: i128,
    pub realized_pnl: i128,
}

impl EvtTradeCancelled {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_TRADE_EXECUTOR),
                Symbol::new(env, "trade_cancelled"),
            ),
            self,
        );
    }
}

// ── oracle ────────────────────────────────────────────────────────────────────

pub const CONTRACT_ORACLE: &str = "oracle";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtOracleRemoved {
    pub oracle: Address,
}

impl EvtOracleRemoved {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_ORACLE),
                Symbol::new(env, "oracle_removed"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtPriceSubmitted {
    pub oracle: Address,
    pub price: i128,
}

impl EvtPriceSubmitted {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_ORACLE),
                Symbol::new(env, "price_submitted"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtConsensusReached {
    pub price: i128,
    pub num_oracles: u32,
}

impl EvtConsensusReached {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_ORACLE),
                Symbol::new(env, "consensus_reached"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtWeightAdjusted {
    pub oracle: Address,
    pub old_weight: u32,
    pub new_weight: u32,
    pub reputation: u32,
}

impl EvtWeightAdjusted {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_ORACLE),
                Symbol::new(env, "weight_adjusted"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtOracleSlashed {
    pub oracle: Address,
    pub penalty: u32,
}

impl EvtOracleSlashed {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_ORACLE),
                Symbol::new(env, "oracle_slashed"),
            ),
            self,
        );
    }
}

// ── signal_registry ───────────────────────────────────────────────────────────

pub const CONTRACT_SIGNAL_REGISTRY: &str = "signal_registry";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSignalAdopted {
    pub signal_id: u64,
    pub adopter: Address,
    pub new_count: u32,
}

impl EvtSignalAdopted {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_SIGNAL_REGISTRY),
                Symbol::new(env, "signal_adopted"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSignalExpired {
    pub signal_id: u64,
    pub provider: Address,
    pub expired_at_ledger: u64,
}

impl EvtSignalExpired {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_SIGNAL_REGISTRY),
                Symbol::new(env, "signal_expired"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTradeExecuted {
    pub signal_id: u64,
    pub executor: Address,
    pub roi: i128,
    pub volume: i128,
}

impl EvtTradeExecuted {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_SIGNAL_REGISTRY),
                Symbol::new(env, "trade_executed"),
            ),
            self,
        );
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtReputationUpdated {
    pub provider: Address,
    pub old_score: u32,
    pub new_score: u32,
}

impl EvtReputationUpdated {
    pub fn publish(self, env: &Env) {
        env.events().publish(
            (
                Symbol::new(env, CONTRACT_SIGNAL_REGISTRY),
                Symbol::new(env, "reputation_updated"),
            ),
            self,
        );
    }
}
