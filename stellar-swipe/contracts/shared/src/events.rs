//! Event deduplication guard (Issue #276).
//!
//! In retry scenarios (e.g. transaction resubmission), the same event could be
//! emitted twice for the same state change. This module provides a lightweight
//! nonce-based guard stored in **temporary storage** (TTL = 1 ledger) so that
//! duplicate emissions within the same ledger are suppressed.
//!
//! # Usage
//!
//! ```rust,ignore
//! use shared::events::{emit_once, EventType};
//!
//! // Only emits if this (event_type, entity_id) has not been emitted this ledger.
//! emit_once(&env, EventType::TradeExecuted, trade_id, || {
//!     env.events().publish((Symbol::new(&env, "trade_executed"),), payload);
//! });
//! ```
//!
//! # Constraint
//!
//! Deduplication is only applied to events that can be emitted multiple times
//! (e.g. `TradeExecuted`, `StopLossTriggered`). One-time events such as
//! `ContractInitialized` are **not** wrapped — they are emitted directly.

use soroban_sdk::{contracttype, Address, Env, Symbol};

/// Discriminant for events that may be emitted more than once per entity.
///
/// Add a new variant here whenever a repeatable event needs deduplication.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventType {
    TradeExecuted,
    StopLossTriggered,
    TakeProfitTriggered,
    SignalAdopted,
    SignalExpired,
    FeeCollected,
}

/// Temporary-storage key for the deduplication nonce.
///
/// Keyed by `(event_type, entity_id)` so different events for the same entity
/// (or the same event for different entities) are tracked independently.
#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    EventNonce(EventType, u64),
}

/// Emit `emit_fn` at most once per `(event_type, entity_id)` per ledger.
///
/// Before emitting: checks `StorageKey::EventNonce(event_type, entity_id)` in
/// temporary storage. If the nonce already exists, the emission is skipped.
/// After emitting: sets the nonce with a TTL of 1 ledger so it expires
/// automatically after the current ledger closes.
///
/// Returns `true` if the event was emitted, `false` if it was deduplicated.
pub fn emit_once<F: FnOnce()>(
    env: &Env,
    event_type: EventType,
    entity_id: u64,
    emit_fn: F,
) -> bool {
    let key = StorageKey::EventNonce(event_type, entity_id);

    if env.storage().temporary().has(&key) {
        // Already emitted this ledger — skip.
        return false;
    }

    emit_fn();

    // Set nonce with TTL = 1 ledger (expires after current ledger closes).
    env.storage().temporary().set(&key, &true);
    env.storage().temporary().extend_ttl(&key, 1, 1);

    true
}

// ── Typed event helpers ───────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct EvtStopLossTriggered {
    pub user: Address,
    pub trade_id: u64,
    pub stop_loss_price: i128,
    pub current_price: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EvtTakeProfitTriggered {
    pub user: Address,
    pub trade_id: u64,
    pub take_profit_price: i128,
    pub current_price: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EvtTradeCancelled {
    pub user: Address,
    pub trade_id: u64,
    pub exit_price: i128,
    pub realized_pnl: i128,
}

pub fn emit_stop_loss_triggered(env: &Env, evt: EvtStopLossTriggered) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "stop_loss_triggered"),
        ),
        evt,
    );
}

pub fn emit_take_profit_triggered(env: &Env, evt: EvtTakeProfitTriggered) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "take_profit_triggered"),
        ),
        evt,
    );
}

pub fn emit_trade_cancelled(env: &Env, evt: EvtTradeCancelled) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "trade_cancelled"),
        ),
        evt,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, symbol_short, testutils::Ledger, Env, Symbol};

    #[contract]
    struct TestContract;

    #[contractimpl]
    impl TestContract {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        env.ledger().with_mut(|l| l.sequence_number = 10);
        let id = env.register(TestContract, ());
        (env, id)
    }

    /// First call emits the event; second call (same ledger) is deduplicated.
    #[test]
    fn test_deduplication_suppresses_second_emission() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            let mut count = 0u32;

            let emitted_first = emit_once(&env, EventType::TradeExecuted, 42, || {
                count += 1;
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 42u64);
            });

            let emitted_second = emit_once(&env, EventType::TradeExecuted, 42, || {
                count += 1;
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 42u64);
            });

            assert!(emitted_first, "first emission must succeed");
            assert!(!emitted_second, "second emission must be deduplicated");
            assert_eq!(count, 1, "emit_fn must be called exactly once");

            // Only one event in the ledger event log.
            assert_eq!(env.events().all().len(), 1);
        });
    }

    /// Different entity_ids are tracked independently — no cross-contamination.
    #[test]
    fn test_different_entity_ids_are_independent() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            let emitted_a = emit_once(&env, EventType::TradeExecuted, 1, || {
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 1u64);
            });

            let emitted_b = emit_once(&env, EventType::TradeExecuted, 2, || {
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 2u64);
            });

            assert!(emitted_a);
            assert!(emitted_b);
            assert_eq!(env.events().all().len(), 2);
        });
    }

    /// Different event types for the same entity_id are tracked independently.
    #[test]
    fn test_different_event_types_are_independent() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            let emitted_trade = emit_once(&env, EventType::TradeExecuted, 99, || {
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 99u64);
            });

            let emitted_stop = emit_once(&env, EventType::StopLossTriggered, 99, || {
                env.events()
                    .publish((Symbol::new(&env, "stop_loss"),), 99u64);
            });

            assert!(emitted_trade);
            assert!(emitted_stop);
            assert_eq!(env.events().all().len(), 2);
        });
    }

    /// Simulates a retry: same (event_type, entity_id) called twice in the same
    /// ledger — only the first emission reaches the event log.
    #[test]
    fn test_retry_scenario_emits_single_event() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            // First attempt (original transaction)
            emit_once(&env, EventType::SignalAdopted, 7, || {
                env.events()
                    .publish((Symbol::new(&env, "signal_adopted"),), 7u64);
            });

            // Retry (resubmitted transaction, same ledger)
            emit_once(&env, EventType::SignalAdopted, 7, || {
                env.events()
                    .publish((Symbol::new(&env, "signal_adopted"),), 7u64);
            });

            // Exactly one event must appear in the log.
            let all_events = env.events().all();
            assert_eq!(all_events.len(), 1, "retry must not produce duplicate event");
        });
    }
}
