//! Standardized event emission for the stake vault (issue #814).
//!
//! Every event published by this crate (including the `emergency_unstake`
//! submodule) follows the workspace two-topic convention documented in
//! `docs/events.md`:
//!
//! ```text
//! topics[0]  contract_name : Symbol  ("stake_vault")
//! topics[1]  event_name    : Symbol  (from `shared::event_topics`)
//! body       <EventStruct>           (a #[contracttype] struct)
//! ```
//!
//! Event-name topics are canonical constants from `shared::event_topics` per
//! issue #585 (`scripts/check_event_topics.sh`). Every event struct carries a
//! `schema_version: u32` field per the versioning policy in
//! `shared::events` / `docs/events.md` — bump `SCHEMA_VERSION` and document
//! the change there on any breaking change to a struct's fields.

use shared::event_topics as topics;
use soroban_sdk::{contracttype, Address, Env, String, Symbol};

/// Current event schema version for this crate's events.
pub const SCHEMA_VERSION: u32 = 1;

/// Topic[0] for every stake-vault event: the full crate name, matching the
/// convention used by every other contract's events module (e.g.
/// `fee_collector`, `trade_executor`, `signal_registry`). Not representable
/// as a short-form `Symbol` constant (11 chars > the 9-char short-symbol
/// limit), so this needs an `Env` and can't live in `event_topics.rs`
/// alongside the other `fn() -> Symbol` constants.
fn contract_topic(env: &Env) -> Symbol {
    Symbol::new(env, "stake_vault")
}

// ── Event structs ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtProviderTierChanged {
    pub schema_version: u32,
    pub provider: Address,
    pub old_tier: u32,
    pub new_tier: u32,
    pub stake_balance: i128,
    pub upgraded: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtMinStakeDurationUpdated {
    pub schema_version: u32,
    pub duration_secs: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtStakeBelowMin {
    pub schema_version: u32,
    pub provider: Address,
    pub current_stake: i128,
    pub minimum: i128,
}

/// Issue #816: configurable withdrawal cooldown.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtWithdrawalCooldownUpdated {
    pub schema_version: u32,
    pub cooldown_secs: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtWithdrawalRequested {
    pub schema_version: u32,
    pub staker: Address,
    pub balance: i128,
    pub unlock_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtFlashLoanAttempt {
    pub schema_version: u32,
    pub staker: Address,
    pub balance: i128,
    pub ledger_seq: u32,
}

/// Issue #816: slash tier limits.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSlashTiersUpdated {
    pub schema_version: u32,
    pub minor_bps: u32,
    pub major_bps: u32,
    pub critical_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtPartialUnstake {
    pub schema_version: u32,
    pub staker: Address,
    pub amount: i128,
    pub remaining: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtStakeSlashed {
    pub schema_version: u32,
    pub provider: Address,
    pub severity: u32,
    pub slash_amount: i128,
    pub slash_id: u64,
    pub reason: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtAppealWindowUpdated {
    pub schema_version: u32,
    pub window_secs: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSlashAppealed {
    pub schema_version: u32,
    pub appellant: Address,
    pub slash_id: u64,
    pub evidence_uri: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtAppealResolved {
    pub schema_version: u32,
    pub slash_id: u64,
    pub uphold: bool,
    pub provider: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtStakeDelegated {
    pub schema_version: u32,
    pub delegator: Address,
    pub provider: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtUnstakeQueued {
    pub schema_version: u32,
    pub staker: Address,
    pub ticket: u64,
    pub queue_position: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtUnstakeProcessed {
    pub schema_version: u32,
    pub staker: Address,
    pub ticket: u64,
    pub amount: i128,
}

/// Issue #815: batch settlement aggregate events.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtBatchSlashCompleted {
    pub schema_version: u32,
    pub processed_count: u32,
    pub total_slashed: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtBatchAppealsResolved {
    pub schema_version: u32,
    pub processed_count: u32,
}

/// Issue #787: lock-duration-weighted voting-power multiplier tiers.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtLockMultiplierTierUpdated {
    pub schema_version: u32,
    pub weeks: u32,
    pub bps: u32,
}

// ── Emergency multi-sig unstake events (issue #754) ───────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtEmergencyConfigured {
    pub schema_version: u32,
    pub required: u32,
    pub penalty_bps: u32,
    pub timeout_secs: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtEmergencyRequested {
    pub schema_version: u32,
    pub staker: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtEmergencyApproved {
    pub schema_version: u32,
    pub staker: Address,
    pub signer: Address,
    pub approvals_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtEmergencyExpired {
    pub schema_version: u32,
    pub staker: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtEmergencyExecuted {
    pub schema_version: u32,
    pub staker: Address,
    pub gross: i128,
    pub penalty: i128,
    pub net: i128,
}

// ── Emit helpers ──────────────────────────────────────────────────────────────

pub fn emit_provider_tier_changed(
    env: &Env,
    provider: Address,
    old_tier: u32,
    new_tier: u32,
    stake_balance: i128,
) {
    let upgraded = new_tier > old_tier;
    let name_topic = if upgraded {
        topics::TOPIC_STAKE_TIER_UP()
    } else {
        topics::TOPIC_STAKE_TIER_DOWN()
    };
    env.events().publish(
        (contract_topic(env), name_topic),
        EvtProviderTierChanged {
            schema_version: SCHEMA_VERSION,
            provider,
            old_tier,
            new_tier,
            stake_balance,
            upgraded,
        },
    );
}

pub fn emit_min_stake_duration_updated(env: &Env, duration_secs: u64) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_MIN_DURATION()),
        EvtMinStakeDurationUpdated {
            schema_version: SCHEMA_VERSION,
            duration_secs,
        },
    );
}

pub fn emit_stake_below_min(env: &Env, provider: Address, current_stake: i128, minimum: i128) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_BELOW_MIN()),
        EvtStakeBelowMin {
            schema_version: SCHEMA_VERSION,
            provider,
            current_stake,
            minimum,
        },
    );
}

pub fn emit_withdrawal_cooldown_updated(env: &Env, cooldown_secs: u64) {
    env.events().publish(
        (
            contract_topic(env),
            topics::TOPIC_STAKE_WITHDRAWAL_COOLDOWN(),
        ),
        EvtWithdrawalCooldownUpdated {
            schema_version: SCHEMA_VERSION,
            cooldown_secs,
        },
    );
}

pub fn emit_withdrawal_requested(env: &Env, staker: Address, balance: i128, unlock_at: u64) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_WITHDRAWAL_REQ()),
        EvtWithdrawalRequested {
            schema_version: SCHEMA_VERSION,
            staker,
            balance,
            unlock_at,
        },
    );
}

pub fn emit_flash_loan_attempt(env: &Env, staker: Address, balance: i128, ledger_seq: u32) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_FLASH_LOAN()),
        EvtFlashLoanAttempt {
            schema_version: SCHEMA_VERSION,
            staker,
            balance,
            ledger_seq,
        },
    );
}

pub fn emit_slash_tiers_updated(env: &Env, minor_bps: u32, major_bps: u32, critical_bps: u32) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_SLASH_TIERS()),
        EvtSlashTiersUpdated {
            schema_version: SCHEMA_VERSION,
            minor_bps,
            major_bps,
            critical_bps,
        },
    );
}

pub fn emit_partial_unstake(env: &Env, staker: Address, amount: i128, remaining: i128) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_PARTIAL_UNSTAKE()),
        EvtPartialUnstake {
            schema_version: SCHEMA_VERSION,
            staker,
            amount,
            remaining,
        },
    );
}

pub fn emit_stake_slashed(
    env: &Env,
    provider: Address,
    severity: u32,
    slash_amount: i128,
    slash_id: u64,
    reason: Symbol,
) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_SLASHED()),
        EvtStakeSlashed {
            schema_version: SCHEMA_VERSION,
            provider,
            severity,
            slash_amount,
            slash_id,
            reason,
        },
    );
}

pub fn emit_appeal_window_updated(env: &Env, window_secs: u64) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_APPEAL_WINDOW()),
        EvtAppealWindowUpdated {
            schema_version: SCHEMA_VERSION,
            window_secs,
        },
    );
}

pub fn emit_slash_appealed(env: &Env, appellant: Address, slash_id: u64, evidence_uri: String) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_APPEALED()),
        EvtSlashAppealed {
            schema_version: SCHEMA_VERSION,
            appellant,
            slash_id,
            evidence_uri,
        },
    );
}

pub fn emit_appeal_resolved(env: &Env, slash_id: u64, uphold: bool, provider: Address) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_APPEAL_RESOLVED()),
        EvtAppealResolved {
            schema_version: SCHEMA_VERSION,
            slash_id,
            uphold,
            provider,
        },
    );
}

pub fn emit_stake_delegated(env: &Env, delegator: Address, provider: Address, amount: i128) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_DELEGATED()),
        EvtStakeDelegated {
            schema_version: SCHEMA_VERSION,
            delegator,
            provider,
            amount,
        },
    );
}

pub fn emit_unstake_queued(env: &Env, staker: Address, ticket: u64, queue_position: u64) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_UNSTAKE_QUEUED()),
        EvtUnstakeQueued {
            schema_version: SCHEMA_VERSION,
            staker,
            ticket,
            queue_position,
        },
    );
}

pub fn emit_unstake_processed(env: &Env, staker: Address, ticket: u64, amount: i128) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_UNSTAKE_PROCESSED()),
        EvtUnstakeProcessed {
            schema_version: SCHEMA_VERSION,
            staker,
            ticket,
            amount,
        },
    );
}

pub fn emit_batch_slash_completed(env: &Env, processed_count: u32, total_slashed: i128) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_BATCH_SLASH()),
        EvtBatchSlashCompleted {
            schema_version: SCHEMA_VERSION,
            processed_count,
            total_slashed,
        },
    );
}

pub fn emit_batch_appeals_resolved(env: &Env, processed_count: u32) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_BATCH_APPEALS()),
        EvtBatchAppealsResolved {
            schema_version: SCHEMA_VERSION,
            processed_count,
        },
    );
}

pub fn emit_lock_multiplier_tier_updated(env: &Env, weeks: u32, bps: u32) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_LOCK_MULTIPLIER()),
        EvtLockMultiplierTierUpdated {
            schema_version: SCHEMA_VERSION,
            weeks,
            bps,
        },
    );
}

pub fn emit_emergency_configured(
    env: &Env,
    required: u32,
    penalty_bps: u32,
    timeout_secs: u64,
) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_EMERGENCY_CFG()),
        EvtEmergencyConfigured {
            schema_version: SCHEMA_VERSION,
            required,
            penalty_bps,
            timeout_secs,
        },
    );
}

pub fn emit_emergency_requested(env: &Env, staker: Address) {
    env.events().publish(
        (contract_topic(env), topics::TOPIC_STAKE_EMERGENCY_REQ()),
        EvtEmergencyRequested {
            schema_version: SCHEMA_VERSION,
            staker,
        },
    );
}

pub fn emit_emergency_approved(env: &Env, staker: Address, signer: Address, approvals_count: u32) {
    env.events().publish(
        (
            contract_topic(env),
            topics::TOPIC_STAKE_EMERGENCY_APPROVED(),
        ),
        EvtEmergencyApproved {
            schema_version: SCHEMA_VERSION,
            staker,
            signer,
            approvals_count,
        },
    );
}

pub fn emit_emergency_expired(env: &Env, staker: Address) {
    env.events().publish(
        (
            contract_topic(env),
            topics::TOPIC_STAKE_EMERGENCY_EXPIRED(),
        ),
        EvtEmergencyExpired {
            schema_version: SCHEMA_VERSION,
            staker,
        },
    );
}

pub fn emit_emergency_executed(env: &Env, staker: Address, gross: i128, penalty: i128, net: i128) {
    env.events().publish(
        (
            contract_topic(env),
            topics::TOPIC_STAKE_EMERGENCY_EXECUTED(),
        ),
        EvtEmergencyExecuted {
            schema_version: SCHEMA_VERSION,
            staker,
            gross,
            penalty,
            net,
        },
    );
}
