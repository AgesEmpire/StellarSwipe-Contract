//! Shared event structs and emit helpers (Issue #275: event versioning).
//!
//! # Versioning policy
//!
//! Every event struct carries a `schema_version: u32` field initialised to `1`.
//!
//! - **Backward-compatible additions** (new optional fields, new events): keep the
//!   same version number.
//! - **Breaking changes** (field removal, type change, field rename): bump
//!   `schema_version` by 1 and document the change in `docs/events.md`.
//!
//! Indexers MUST check `schema_version` before deserialising event bodies so they
//! can handle multiple schema generations gracefully.
//!
//! # Event deduplication guard (Issue #276)
//!
//! In retry scenarios the same event could be emitted twice for the same state
//! change. [`emit_once`] provides a lightweight nonce-based guard stored in
//! **temporary storage** (TTL = 1 ledger) that suppresses duplicate emissions
//! within the same ledger.

use soroban_sdk::{contracttype, Address, Env, String, Symbol};

// ── Schema version constant ───────────────────────────────────────────────────

/// Current event schema version. Bump when making breaking changes to any event struct.
pub const SCHEMA_VERSION: u32 = 1;

// ── Event structs ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTradeCancelled {
    pub schema_version: u32,
    pub user: Address,
    pub trade_id: u64,
    pub exit_price: i128,
    pub realized_pnl: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtStopLossTriggered {
    pub schema_version: u32,
    pub user: Address,
    pub trade_id: u64,
    pub stop_loss_price: i128,
    pub current_price: i128,
    /// Always `true` — user must review their position after a stop-loss.
    pub action_required: bool,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTakeProfitTriggered {
    pub schema_version: u32,
    pub user: Address,
    pub trade_id: u64,
    pub take_profit_price: i128,
    pub current_price: i128,
    /// Always `true` — user should confirm the closed position and realised P&L.
    pub action_required: bool,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTradeShareable {
    pub schema_version: u32,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtPositionClosedByKeeper {
    pub schema_version: u32,
    pub user: Address,
    pub position_id: u64,
    pub asset_pair: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSubscriptionCreated {
    pub schema_version: u32,
    pub user: Address,
    pub provider: Address,
    pub expires_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSignalAdopted {
    pub schema_version: u32,
    pub signal_id: u64,
    pub adopter: Address,
    pub new_count: u32,
    /// Address of the user who adopted the signal.
    pub user: Address,
    pub timestamp: u64,
    /// `false` — signal adoption is informational, no user action required.
    pub action_required: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtPositionClosed {
    pub schema_version: u32,
    pub user: Address,
    pub trade_id: u64,
    pub exit_price: i128,
    pub realized_pnl: i128,
    pub timestamp: u64,
    /// `false` — position closure is informational; no further action required.
    pub action_required: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSignalEdited {
    pub schema_version: u32,
    pub signal_id: u64,
    pub provider: Address,
    pub price: i128,
    pub rationale_hash: String,
    pub confidence: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtReputationUpdated {
    pub schema_version: u32,
    pub provider: Address,
    pub old_score: u32,
    pub new_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtStakeChanged {
    pub schema_version: u32,
    pub holder: Address,
    pub amount: i128,
    pub is_stake: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtRewardClaimed {
    pub schema_version: u32,
    pub beneficiary: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtVestingReleased {
    pub schema_version: u32,
    pub beneficiary: Address,
    pub amount: i128,
}

// ── Emit helpers ──────────────────────────────────────────────────────────────

pub fn emit_trade_cancelled(env: &Env, evt: EvtTradeCancelled) {
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "trade_cancelled"),
        ),
        evt,
    );
}

pub fn emit_stop_loss_triggered(env: &Env, evt: EvtStopLossTriggered) {
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "stop_loss_triggered"),
        ),
        evt,
    );
}

pub fn emit_take_profit_triggered(env: &Env, evt: EvtTakeProfitTriggered) {
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "take_profit_triggered"),
        ),
        evt,
    );
}

pub fn emit_trade_shareable(env: &Env, evt: EvtTradeShareable) {
    env.events().publish(
        (
            Symbol::new(env, "user_portfolio"),
            Symbol::new(env, "trade_shareable"),
        ),
        evt,
    );
}

pub fn emit_position_closed_by_keeper(env: &Env, evt: EvtPositionClosedByKeeper) {
    env.events().publish(
        (
            Symbol::new(env, "user_portfolio"),
            Symbol::new(env, "keeper_close"),
        ),
        evt,
    );
}

pub fn emit_subscription_created(env: &Env, evt: EvtSubscriptionCreated) {
    env.events().publish(
        (
            Symbol::new(env, "user_portfolio"),
            Symbol::new(env, "subscription_created"),
        ),
        evt,
    );
}

pub fn emit_signal_adopted(env: &Env, evt: EvtSignalAdopted) {
    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "signal_adopted"),
        ),
        evt,
    );
}

pub fn emit_position_closed(env: &Env, evt: EvtPositionClosed) {
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "position_closed"),
        ),
        evt,
    );
}

pub fn emit_signal_edited(env: &Env, evt: EvtSignalEdited) {
    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "signal_edited"),
        ),
        evt,
    );
}

pub fn emit_reputation_updated(env: &Env, evt: EvtReputationUpdated) {
    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "reputation_updated"),
        ),
        evt,
    );
}

pub fn emit_stake_changed(env: &Env, evt: EvtStakeChanged) {
    env.events().publish(
        (
            Symbol::new(env, "governance"),
            Symbol::new(env, "stake_changed"),
        ),
        evt,
    );
}

pub fn emit_reward_claimed(env: &Env, evt: EvtRewardClaimed) {
    env.events().publish(
        (
            Symbol::new(env, "governance"),
            Symbol::new(env, "reward_claimed"),
        ),
        evt,
    );
}

pub fn emit_vesting_released(env: &Env, evt: EvtVestingReleased) {
    env.events().publish(
        (
            Symbol::new(env, "governance"),
            Symbol::new(env, "vesting_released"),
        ),
        evt,
    );
}

// ── Geographic restriction event structs ─────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtUserRestricted {
    pub schema_version: u32,
    pub user: Address,
    /// IPFS hash of the restriction reason document. No reason text stored on-chain.
    pub reason_hash: String,
    pub restricted: bool,
}

pub fn emit_user_restricted(env: &Env, evt: EvtUserRestricted) {
    env.events().publish(
        (
            Symbol::new(env, "user_portfolio"),
            Symbol::new(env, "user_restricted"),
        ),
        evt,
    );
}

// ── KYC event structs ─────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtKycStatusUpdated {
    pub schema_version: u32,
    pub user: Address,
    pub verified: bool,
}

pub fn emit_kyc_status_updated(env: &Env, evt: EvtKycStatusUpdated) {
    env.events().publish(
        (
            Symbol::new(env, "user_portfolio"),
            Symbol::new(env, "kyc_status_updated"),
        ),
        evt,
    );
}

// ── Analytics event structs (Issue #365) ─────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtUserSessionStarted {
    pub schema_version: u32,
    pub user: Address,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSignalViewed {
    pub schema_version: u32,
    pub user: Address,
    pub signal_id: u64,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtSignalSwiped {
    pub schema_version: u32,
    pub user: Address,
    pub signal_id: u64,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtTradeExecuted {
    pub schema_version: u32,
    pub user: Address,
    pub signal_id: u64,
    pub timestamp: u64,
}

// ── Analytics emit helpers (Issue #365) ──────────────────────────────────────

pub fn emit_user_session_started(env: &Env, evt: EvtUserSessionStarted) {
    env.events().publish(
        (
            Symbol::new(env, "analytics"),
            Symbol::new(env, "session_started"),
        ),
        evt,
    );
}

pub fn emit_signal_viewed(env: &Env, evt: EvtSignalViewed) {
    env.events().publish(
        (
            Symbol::new(env, "analytics"),
            Symbol::new(env, "signal_viewed"),
        ),
        evt,
    );
}

pub fn emit_signal_swiped(env: &Env, evt: EvtSignalSwiped) {
    env.events().publish(
        (
            Symbol::new(env, "analytics"),
            Symbol::new(env, "signal_swiped"),
        ),
        evt,
    );
}

pub fn emit_analytics_trade_executed(env: &Env, evt: EvtTradeExecuted) {
    env.events().publish(
        (
            Symbol::new(env, "analytics"),
            Symbol::new(env, "trade_executed"),
        ),
        evt,
    );
}

// ── Event deduplication guard ─────────────────────────────────────────────────

/// Discriminant for events that may be emitted more than once per entity.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventType {
    TradeExecuted,
    StopLossTriggered,
    TakeProfitTriggered,
    SignalAdopted,
    SignalExpired,
    FeeCollected,
    UserSession,
}

/// Temporary-storage key for the deduplication nonce.
#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    EventNonce(EventType, u64),
    /// Per-user session guard: set in temporary storage for SESSION_TTL_LEDGERS to prevent
    /// re-emitting UserSessionStarted within the same session window.
    UserSession(Address),
}

/// Session window in ledgers (~10 minutes at 5 s/ledger).
pub const SESSION_TTL_LEDGERS: u32 = 120;

/// Emit `UserSessionStarted` for `user` at most once per session window.
///
/// Uses temporary storage keyed by `StorageKey::UserSession(user)` with a
/// `SESSION_TTL_LEDGERS` TTL so repeated calls within the same session are
/// suppressed. No business-logic state is changed.
pub fn emit_session_started_once(env: &Env, user: &Address) {
    let key = StorageKey::UserSession(user.clone());
    if env.storage().temporary().has(&key) {
        return;
    }
    emit_user_session_started(
        env,
        EvtUserSessionStarted {
            schema_version: SCHEMA_VERSION,
            user: user.clone(),
            timestamp: env.ledger().timestamp(),
        },
    );
    env.storage().temporary().set(&key, &true);
    env.storage()
        .temporary()
        .extend_ttl(&key, SESSION_TTL_LEDGERS, SESSION_TTL_LEDGERS);
}

/// Emit `emit_fn` at most once per `(event_type, entity_id)` per ledger.
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
        return false;
    }

    emit_fn();

    env.storage().temporary().set(&key, &true);
    env.storage().temporary().extend_ttl(&key, 1, 1);

    true
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, testutils::{Address as _, Events, Ledger}, Env};

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

    // ── schema_version field tests ────────────────────────────────────────────

    #[test]
    fn evt_trade_cancelled_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtTradeCancelled {
            schema_version: SCHEMA_VERSION,
            user: addr,
            trade_id: 1,
            exit_price: 100,
            realized_pnl: 10,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_stop_loss_triggered_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtStopLossTriggered {
            schema_version: SCHEMA_VERSION,
            user: addr,
            trade_id: 1,
            stop_loss_price: 90,
            current_price: 85,
            action_required: true,
            timestamp: 1000,
        };
        assert_eq!(evt.schema_version, 1);
        assert!(evt.action_required);
    }

    #[test]
    fn evt_take_profit_triggered_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtTakeProfitTriggered {
            schema_version: SCHEMA_VERSION,
            user: addr,
            trade_id: 1,
            take_profit_price: 120,
            current_price: 125,
            action_required: true,
            timestamp: 1000,
        };
        assert_eq!(evt.schema_version, 1);
        assert!(evt.action_required);
    }

    #[test]
    fn evt_trade_shareable_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let provider = soroban_sdk::Address::generate(&env);
        let evt = EvtTradeShareable {
            schema_version: SCHEMA_VERSION,
            user: addr,
            position_id: 1,
            asset_pair: 7,
            entry_price: 100,
            exit_price: 120,
            pnl_bps: 2000,
            signal_provider: provider,
            signal_id: 42,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_position_closed_by_keeper_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtPositionClosedByKeeper {
            schema_version: SCHEMA_VERSION,
            user: addr,
            position_id: 1,
            asset_pair: 7,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_subscription_created_has_schema_version() {
        let env = Env::default();
        let user = soroban_sdk::Address::generate(&env);
        let provider = soroban_sdk::Address::generate(&env);
        let evt = EvtSubscriptionCreated {
            schema_version: SCHEMA_VERSION,
            user,
            provider,
            expires_at: 9999,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_signal_adopted_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtSignalAdopted {
            schema_version: SCHEMA_VERSION,
            signal_id: 1,
            adopter: addr.clone(),
            new_count: 5,
            user: addr,
            timestamp: 2000,
            action_required: false,
        };
        assert_eq!(evt.schema_version, 1);
        assert!(!evt.action_required);
    }

    #[test]
    fn evt_position_closed_has_required_fields() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtPositionClosed {
            schema_version: SCHEMA_VERSION,
            user: addr,
            trade_id: 42,
            exit_price: 150,
            realized_pnl: 50,
            timestamp: 3000,
            action_required: false,
        };
        assert_eq!(evt.schema_version, 1);
        assert_eq!(evt.trade_id, 42);
        assert!(!evt.action_required);
    }

    #[test]
    fn evt_signal_edited_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtSignalEdited {
            schema_version: SCHEMA_VERSION,
            signal_id: 1,
            provider: addr,
            price: 100,
            rationale_hash: soroban_sdk::String::from_str(&env, "abc"),
            confidence: 80,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_reputation_updated_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtReputationUpdated {
            schema_version: SCHEMA_VERSION,
            provider: addr,
            old_score: 50,
            new_score: 60,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_stake_changed_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtStakeChanged {
            schema_version: SCHEMA_VERSION,
            holder: addr,
            amount: 1000,
            is_stake: true,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_reward_claimed_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtRewardClaimed {
            schema_version: SCHEMA_VERSION,
            beneficiary: addr,
            amount: 500,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_vesting_released_has_schema_version() {
        let env = Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        let evt = EvtVestingReleased {
            schema_version: SCHEMA_VERSION,
            beneficiary: addr,
            amount: 200,
        };
        assert_eq!(evt.schema_version, 1);
    }

    // ── Deduplication tests ───────────────────────────────────────────────────

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

            assert!(emitted_first);
            assert!(!emitted_second);
            assert_eq!(count, 1);
            assert_eq!(env.events().all().len(), 1);
        });
    }

    #[test]
    fn test_different_entity_ids_are_independent() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            let a = emit_once(&env, EventType::TradeExecuted, 1, || {
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 1u64);
            });
            let b = emit_once(&env, EventType::TradeExecuted, 2, || {
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 2u64);
            });
            assert!(a);
            assert!(b);
            assert_eq!(env.events().all().len(), 2);
        });
    }

    #[test]
    fn test_different_event_types_are_independent() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            let a = emit_once(&env, EventType::TradeExecuted, 99, || {
                env.events()
                    .publish((Symbol::new(&env, "trade_executed"),), 99u64);
            });
            let b = emit_once(&env, EventType::StopLossTriggered, 99, || {
                env.events()
                    .publish((Symbol::new(&env, "stop_loss"),), 99u64);
            });
            assert!(a);
            assert!(b);
            assert_eq!(env.events().all().len(), 2);
        });
    }

    // ── Analytics event struct tests (Issue #365) ────────────────────────────

    #[test]
    fn evt_user_session_started_has_schema_version() {
        let env = Env::default();
        let user = soroban_sdk::Address::generate(&env);
        let evt = EvtUserSessionStarted {
            schema_version: SCHEMA_VERSION,
            user,
            timestamp: 12345,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn evt_signal_viewed_has_schema_version() {
        let env = Env::default();
        let user = soroban_sdk::Address::generate(&env);
        let evt = EvtSignalViewed {
            schema_version: SCHEMA_VERSION,
            user,
            signal_id: 42,
            timestamp: 12345,
        };
        assert_eq!(evt.schema_version, 1);
        assert_eq!(evt.signal_id, 42);
    }

    #[test]
    fn evt_signal_swiped_has_schema_version() {
        let env = Env::default();
        let user = soroban_sdk::Address::generate(&env);
        let evt = EvtSignalSwiped {
            schema_version: SCHEMA_VERSION,
            user,
            signal_id: 7,
            timestamp: 99999,
        };
        assert_eq!(evt.schema_version, 1);
        assert_eq!(evt.signal_id, 7);
    }

    #[test]
    fn evt_trade_executed_has_schema_version() {
        let env = Env::default();
        let user = soroban_sdk::Address::generate(&env);
        let evt = EvtTradeExecuted {
            schema_version: SCHEMA_VERSION,
            user,
            signal_id: 1,
            timestamp: 5000,
        };
        assert_eq!(evt.schema_version, 1);
    }

    #[test]
    fn emit_session_started_once_emits_on_first_call() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            emit_session_started_once(&env, &user);
            let all = env.events().all();
            assert_eq!(all.len(), 1);
        });
    }

    #[test]
    fn emit_session_started_once_deduplicates_within_session() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            emit_session_started_once(&env, &user);
            emit_session_started_once(&env, &user);
            emit_session_started_once(&env, &user);
            // Only 1 event emitted despite 3 calls
            assert_eq!(env.events().all().len(), 1);
        });
    }

    #[test]
    fn emit_signal_viewed_emits_event_with_user_and_signal_id() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            emit_signal_viewed(
                &env,
                EvtSignalViewed {
                    schema_version: SCHEMA_VERSION,
                    user: user.clone(),
                    signal_id: 55,
                    timestamp: env.ledger().timestamp(),
                },
            );
            assert_eq!(env.events().all().len(), 1);
        });
    }

    #[test]
    fn emit_signal_swiped_emits_event_with_user_and_signal_id() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            emit_signal_swiped(
                &env,
                EvtSignalSwiped {
                    schema_version: SCHEMA_VERSION,
                    user: user.clone(),
                    signal_id: 3,
                    timestamp: env.ledger().timestamp(),
                },
            );
            assert_eq!(env.events().all().len(), 1);
        });
    }

    #[test]
    fn emit_analytics_trade_executed_emits_event() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            emit_analytics_trade_executed(
                &env,
                EvtTradeExecuted {
                    schema_version: SCHEMA_VERSION,
                    user: user.clone(),
                    signal_id: 9,
                    timestamp: env.ledger().timestamp(),
                },
            );
            assert_eq!(env.events().all().len(), 1);
        });
    }

    #[test]
    fn test_retry_scenario_emits_single_event() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            emit_once(&env, EventType::SignalAdopted, 7, || {
                env.events()
                    .publish((Symbol::new(&env, "signal_adopted"),), 7u64);
            });
            emit_once(&env, EventType::SignalAdopted, 7, || {
                env.events()
                    .publish((Symbol::new(&env, "signal_adopted"),), 7u64);
            });
            assert_eq!(env.events().all().len(), 1);
        });
    }
}
