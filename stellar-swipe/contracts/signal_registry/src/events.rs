use crate::types::{Asset, MigrationProgress};
use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};

// Horizon / indexer: first topic is only the event name (ScVal::Symbol);
// all identifying fields live in a standard ScVal body (tuple or #[contracttype]).

pub fn emit_admin_transfer_proposed(
    env: &Env,
    current_admin: Address,
    pending_admin: Address,
    expires_at: u64,
) {
    let topics = (Symbol::new(env, "admin_transfer_proposed"),);
    env.events()
        .publish(topics, (current_admin, pending_admin, expires_at));
}

pub fn emit_admin_transfer_completed(env: &Env, old_admin: Address, new_admin: Address) {
    let topics = (Symbol::new(env, "admin_transfer_completed"),);
    env.events().publish(topics, (old_admin, new_admin));
}

pub fn emit_admin_transferred(env: &Env, old_admin: Address, new_admin: Address) {
    let topics = (Symbol::new(env, "admin_transferred"),);
    env.events().publish(topics, (old_admin, new_admin));
}

pub fn emit_parameter_updated(env: &Env, parameter: Symbol, old_value: i128, new_value: i128) {
    let topics = (Symbol::new(env, "parameter_updated"),);
    env.events()
        .publish(topics, (parameter, old_value, new_value));
}

pub fn emit_trading_paused(env: &Env, paused_by: Address, expires_at: u64) {
    let topics = (Symbol::new(env, "trading_paused"),);
    let timestamp = env.ledger().timestamp();
    env.events()
        .publish(topics, (paused_by, timestamp, expires_at));
}

pub fn emit_trading_unpaused(env: &Env, unpaused_by: Address) {
    let topics = (Symbol::new(env, "trading_unpaused"),);
    let timestamp = env.ledger().timestamp();
    env.events().publish(topics, (unpaused_by, timestamp));
}

pub fn emit_multisig_signer_added(env: &Env, signer: Address, added_by: Address) {
    let topics = (Symbol::new(env, "multisig_signer_added"),);
    env.events().publish(topics, (signer, added_by));
}

pub fn emit_multisig_signer_removed(env: &Env, signer: Address, removed_by: Address) {
    let topics = (Symbol::new(env, "multisig_signer_removed"),);
    env.events().publish(topics, (signer, removed_by));
}

pub fn emit_fee_collected(
    env: &Env,
    asset: Asset,
    total_fee: i128,
    platform_fee: i128,
    provider_fee: i128,
    provider: Address,
    platform_treasury: Address,
) {
    let topics = (Symbol::new(env, "fee_collected"),);
    env.events().publish(
        topics,
        (
            asset,
            total_fee,
            platform_fee,
            provider_fee,
            provider,
            platform_treasury,
        ),
    );
}

#[contracttype]
#[derive(Clone)]
pub struct SignalAdoptedEvent {
    pub signal_id: u64,
    pub adopter: Address,
    pub new_count: u32,
}

pub fn emit_signal_adopted(env: &Env, signal_id: u64, adopter: Address, new_count: u32) {
    shared::events::emit_signal_adopted(
        env,
        shared::events::EvtSignalAdopted {
            schema_version: shared::events::SCHEMA_VERSION,
            signal_id,
            adopter: adopter.clone(),
            new_count,
            user: adopter,
            timestamp: env.ledger().timestamp(),
            action_required: false,
        },
    );
}

pub fn emit_signal_expired(env: &Env, signal_id: u64, provider: Address, expired_at_ledger: u64) {
    let topics = (Symbol::new(env, "signal_expired"),);
    env.events()
        .publish(topics, (signal_id, provider, expired_at_ledger));
}

pub fn emit_trade_executed(env: &Env, signal_id: u64, executor: Address, roi: i128, volume: i128) {
    let topics = (Symbol::new(env, "trade_executed"),);
    env.events()
        .publish(topics, (signal_id, executor, roi, volume));
}

pub fn emit_signal_status_changed(
    env: &Env,
    signal_id: u64,
    provider: Address,
    old_status: u32,
    new_status: u32,
) {
    let topics = (Symbol::new(env, "signal_status_changed"),);
    env.events()
        .publish(topics, (signal_id, provider, old_status, new_status));
}

pub fn emit_provider_stats_updated(
    env: &Env,
    provider: Address,
    success_rate: u32,
    avg_return: i128,
    total_volume: i128,
) {
    let topics = (Symbol::new(env, "provider_stats_updated"),);
    env.events()
        .publish(topics, (provider, success_rate, avg_return, total_volume));
}

pub fn emit_follow_gained(env: &Env, user: Address, provider: Address, new_count: u32) {
    let topics = (Symbol::new(env, "follow_gained"),);
    env.events()
        .publish(topics, (user, provider, new_count));
}

pub fn emit_follow_lost(env: &Env, user: Address, provider: Address, new_count: u32) {
    let topics = (Symbol::new(env, "follow_lost"),);
    env.events()
        .publish(topics, (user, provider, new_count));
}

pub fn emit_tags_added(env: &Env, signal_id: u64, provider: Address, tag_count: u32) {
    let topics = (Symbol::new(env, "tags_added"),);
    env.events()
        .publish(topics, (signal_id, provider, tag_count));
}

pub fn emit_collaborative_signal_created(env: &Env, signal_id: u64, authors: Vec<Address>) {
    let topics = (Symbol::new(env, "collab_signal_created"),);
    env.events().publish(topics, (signal_id, authors));
}

pub fn emit_collaborative_signal_approved(env: &Env, signal_id: u64, approver: Address) {
    let topics = (Symbol::new(env, "collab_signal_approved"),);
    env.events()
        .publish(topics, (signal_id, approver));
}

pub fn emit_collaborative_signal_published(env: &Env, signal_id: u64) {
    let topics = (Symbol::new(env, "collab_signal_published"),);
    env.events().publish(topics, signal_id);
}

pub fn emit_data_exported(env: &Env, requester: Address, entity_type: u32, record_count: u32) {
    let topics = (Symbol::new(env, "data_exported"),);
    env.events()
        .publish(topics, (requester, entity_type, record_count));
}

pub fn emit_combo_created(env: &Env, combo_id: u64, provider: Address, component_count: u32) {
    let topics = (Symbol::new(env, "combo_created"),);
    env.events()
        .publish(topics, (combo_id, provider, component_count));
}

pub fn emit_combo_executed(env: &Env, combo_id: u64, executor: Address, combined_roi: i128) {
    let topics = (Symbol::new(env, "combo_executed"),);
    env.events()
        .publish(topics, (combo_id, executor, combined_roi));
}

pub fn emit_combo_cancelled(env: &Env, combo_id: u64, provider: Address) {
    let topics = (Symbol::new(env, "combo_cancelled"),);
    env.events().publish(topics, (combo_id, provider));
}

pub fn emit_signal_updated(env: &Env, signal_id: u64, version: u32, updater: Address) {
    let topics = (Symbol::new(env, "signal_updated"),);
    env.events()
        .publish(topics, (signal_id, version, updater));
}

#[contracttype]
#[derive(Clone)]
pub struct SignalEditedEvent {
    pub signal_id: u64,
    pub provider: Address,
    pub price: i128,
    pub rationale_hash: String,
    pub confidence: u32,
}

pub fn emit_signal_edited(
    env: &Env,
    signal_id: u64,
    provider: Address,
    price: i128,
    rationale_hash: String,
    confidence: u32,
) {
    shared::events::emit_signal_edited(
        env,
        shared::events::EvtSignalEdited {
            schema_version: shared::events::SCHEMA_VERSION,
            signal_id,
            provider,
            price,
            rationale_hash,
            confidence,
        },
    );
}

pub fn emit_copy_recorded(env: &Env, user: Address, signal_id: u64, version: u32) {
    let topics = (Symbol::new(env, "copy_recorded"),);
    env.events()
        .publish(topics, (user, signal_id, version));
}

pub fn emit_cross_chain_signal_requested(
    env: &Env,
    source_chain: soroban_sdk::String,
    source_id: soroban_sdk::String,
    provider: Address,
) {
    let topics = (Symbol::new(env, "cross_chain_requested"),);
    env.events()
        .publish(topics, (source_chain, source_id, provider));
}

pub fn emit_cross_chain_signal_imported(
    env: &Env,
    source_chain: soroban_sdk::String,
    source_id: soroban_sdk::String,
    stellar_id: u64,
) {
    let topics = (Symbol::new(env, "cross_chain_imported"),);
    env.events()
        .publish(topics, (source_chain, source_id, stellar_id));
}

pub fn emit_cross_chain_address_registered(
    env: &Env,
    source_chain: soroban_sdk::String,
    source_address: soroban_sdk::String,
    stellar_address: Address,
) {
    let topics = (Symbol::new(env, "cross_chain_address_registered"),);
    env.events()
        .publish(topics, (source_chain, source_address, stellar_address));
}

pub fn emit_cross_chain_signal_synced(
    env: &Env,
    source_chain: soroban_sdk::String,
    source_id: soroban_sdk::String,
    new_status: u32,
) {
    let topics = (Symbol::new(env, "cross_chain_synced"),);
    env.events()
        .publish(topics, (source_chain, source_id, new_status));
}

pub fn emit_emergency_paused(
    env: &Env,
    category: String,
    paused_by: Address,
    reason: String,
    auto_unpause_at: Option<u64>,
) {
    let topics = (Symbol::new(env, "emergency_paused"),);
    env.events()
        .publish(topics, (category, paused_by, reason, auto_unpause_at));
}

pub fn emit_emergency_unpaused(env: &Env, category: String, unpaused_by: Address) {
    let topics = (Symbol::new(env, "emergency_unpaused"),);
    env.events()
        .publish(topics, (category, unpaused_by));
}

pub fn emit_circuit_breaker_triggered(env: &Env, category: String, reason: String) {
    let topics = (Symbol::new(env, "circuit_breaker_triggered"),);
    env.events()
        .publish(topics, (category, reason));
}

pub fn emit_guardian_set(env: &Env, guardian: Address) {
    let topics = (Symbol::new(env, "guardian_set"),);
    env.events().publish(topics, guardian);
}

pub fn emit_guardian_revoked(env: &Env, guardian: Address) {
    let topics = (Symbol::new(env, "guardian_revoked"),);
    env.events().publish(topics, guardian);
}

#[contracttype]
#[derive(Clone)]
pub struct ReputationUpdatedEvent {
    pub provider: Address,
    pub old_score: u32,
    pub new_score: u32,
}

pub fn emit_reputation_updated(env: &Env, provider: Address, old_score: u32, new_score: u32) {
    shared::events::emit_reputation_updated(
        env,
        shared::events::EvtReputationUpdated {
            schema_version: shared::events::SCHEMA_VERSION,
            provider,
            old_score,
            new_score,
        },
    );
}
