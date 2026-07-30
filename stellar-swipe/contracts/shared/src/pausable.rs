/// Shared pausable module (Issue #561).
///
/// Provides a consistent pause-state storage key, pause/unpause helpers, a
/// "reject if paused" guard, and a uniform event emitted on every state
/// change.  Any contract that needs an emergency-pause capability imports
/// these helpers instead of rolling its own.
///
/// # Scoped pause (Issue: per-contract pause scope granularity)
///
/// In addition to the global `PausableKey::Paused` flag, contracts can use
/// per-scope pause keys stored under [`ScopedPausableStorageKey`].  This
/// lets governance isolate incidents to high-risk contract groups (treasury,
/// staking, trading, upgrades) without disrupting unrelated flows.
///
/// # Migration from bespoke pause logic
/// A contract that already stores a paused flag under a different key can
/// migrate without losing its current pause status by running a one-time
/// upgrade that reads the old key and writes it to [`PausableKey::Paused`]:
///
/// ```ignore
/// // Inside an upgrade entrypoint (or a one-time migration call):
/// let was_paused: bool = env.storage().instance()
///     .get(&OldPauseKey::Paused)          // old enum variant / key
///     .unwrap_or(false);
/// shared::pausable::set_paused(&env, was_paused);
/// env.storage().instance().remove(&OldPauseKey::Paused);  // clean up old key
/// ```
///
/// After migration the old key is no longer consulted; only
/// [`PausableKey::Paused`] is read.
use soroban_sdk::{contractclient, contracttype, symbol_short, Env, Symbol};

// ── Global pause ───────────────────────────────────────────────────────────────

/// Storage key for the pause flag.  Defined as a distinct enum so it cannot
/// collide with contract-local key enums whose variants have different
/// discriminants.
#[contracttype]
#[derive(Clone)]
pub enum PausableKey {
    Paused,
}

/// Returns `true` when the contract is globally paused.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<_, bool>(&PausableKey::Paused)
        .unwrap_or(false)
}

/// Persists the new global pause state and emits a consistent event.
///
/// Event topic  : `("contract_paused",)`  or  `("contract_unpaused",)`
/// Event data   : `(paused: bool)`
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&PausableKey::Paused, &paused);

    let topic: Symbol = if paused {
        symbol_short!("paused")
    } else {
        symbol_short!("unpaused")
    };
    #[allow(deprecated)]
    env.events().publish((topic,), (paused,));
}

/// Returns `Err(true)` (a sentinel) if the contract is currently globally paused,
/// `Ok(())` otherwise.
///
/// Callers translate the boolean sentinel into their own error type:
///
/// ```ignore
/// shared::pausable::require_not_paused(&env)
///     .map_err(|_| MyError::ContractPaused)?;
/// ```
pub fn require_not_paused(env: &Env) -> Result<(), bool> {
    if is_paused(env) {
        Err(true)
    } else {
        Ok(())
    }
}

// ── Scoped pause (Issue: per-contract pause scope granularity) ─────────────────

/// High-risk contract surface areas that can be paused independently.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PauseScope {
    Global = 0,
    Treasury = 1,
    Staking = 2,
    Governance = 3,
    Trading = 4,
    Upgrades = 5,
}

/// Storage key for a scoped pause flag.  Distinct from [`PausableKey`] so
/// existing contracts that only use the global flag need no migration.
#[contracttype]
#[derive(Clone)]
pub enum ScopedPausableStorageKey {
    Paused(PauseScope),
}

/// Return `true` when `scope` is currently paused.
///
/// - `PauseScope::Global` delegates to [`is_paused`].
/// - Other scopes consult the scoped storage key.
pub fn is_paused_for_scope(env: &Env, scope: PauseScope) -> bool {
    match scope {
        PauseScope::Global => is_paused(env),
        _ => env
            .storage()
            .instance()
            .get::<_, bool>(&ScopedPausableStorageKey::Paused(scope))
            .unwrap_or(false),
    }
}

/// Persist the pause state for `scope` and emit an event on the
/// `("pause_scope",)` topic.
///
/// Event data : `(scope_code: u32, paused: bool)`
pub fn set_paused_scope(env: &Env, paused: bool, scope: PauseScope) {
    match scope {
        PauseScope::Global => set_paused(env, paused),
        _ => {
            env.storage()
                .instance()
                .set(&ScopedPausableStorageKey::Paused(scope), &paused);

            #[allow(deprecated)]
            env.events().publish(
                (symbol_short!("pause_scope"),),
                ((scope as u32), paused),
            );
        }
    }
}

/// Return `Err(scope)` (a sentinel) if `scope` is currently paused,
/// `Ok(())` otherwise.
///
/// Callers translate the scope sentinel into their own error type:
///
/// ```ignore
/// shared::pausable::require_not_paused_scope(&env, PauseScope::Treasury)
///     .map_err(|_| MyError::TreasuryPaused)?;
/// ```
pub fn require_not_paused_scope(
    env: &Env,
    scope: PauseScope,
) -> Result<(), PauseScope> {
    if is_paused_for_scope(env, scope) {
        Err(scope)
    } else {
        Ok(())
    }
}

// ── Cross-contract governance pause propagation ────────────────────────────────

/// Cross-contract client for governance-driven pause propagation (Issue #865).
///
/// Downstream contracts implement `apply_governance_pause` so a central
/// governance contract can push a pause/unpause to every registered
/// downstream contract with a single, uniform call regardless of that
/// contract's own internal pause representation. Implementations are
/// expected to authorize the call by requiring their configured governance
/// address (stored locally) rather than requiring the transaction signer,
/// since this is a contract-to-contract call.
#[contractclient(name = "PausableClient")]
pub trait PausableTrait {
    fn apply_governance_pause(env: Env, paused: bool);
}

#[contractclient(name = "PausableScopedClient")]
pub trait PausableScopedTrait {
    fn apply_governance_pause_scope(env: Env, paused: bool, scope: u32);
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, contractimpl, Address, Env};

    #[contract]
    struct TestPausable;

    #[contractimpl]
    impl TestPausable {
        pub fn pause(env: Env) {
            set_paused(&env, true);
        }
        pub fn unpause(env: Env) {
            set_paused(&env, false);
        }
        pub fn paused(env: Env) -> bool {
            is_paused(&env)
        }
        pub fn guard(env: Env) -> bool {
            require_not_paused(&env).is_err()
        }
        pub fn pause_scope(env: Env, scope: PauseScope) {
            set_paused_scope(&env, true, scope);
        }
        pub fn unpause_scope(env: Env, scope: PauseScope) {
            set_paused_scope(&env, false, scope);
        }
        pub fn paused_scope(env: Env, scope: PauseScope) -> bool {
            is_paused_for_scope(&env, scope)
        }
        pub fn guard_scope(env: Env, scope: PauseScope) -> bool {
            require_not_paused_scope(&env, scope).is_err()
        }
    }

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(TestPausable, ());
        (env, id)
    }

    // ── Global pause tests ─────────────────────────────────────────────────────

    #[test]
    fn fresh_contract_is_not_paused() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);
        assert!(!c.paused());
    }

    #[test]
    fn pause_sets_flag() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);
        c.pause();
        assert!(c.paused());
    }

    #[test]
    fn unpause_clears_flag() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);
        c.pause();
        c.unpause();
        assert!(!c.paused());
    }

    #[test]
    fn guard_returns_err_when_paused() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);
        c.pause();
        assert!(c.guard(), "guard must return Err (true) when paused");
    }

    #[test]
    fn guard_returns_ok_when_not_paused() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);
        assert!(!c.guard(), "guard must return Ok when not paused");
    }

    #[test]
    fn pause_emits_event() {
        use soroban_sdk::testutils::Events;
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);
        c.pause();
        let events = env.events().all();
        assert!(!events.is_empty(), "pause must emit an event");
    }

    #[test]
    fn unpause_emits_event() {
        use soroban_sdk::testutils::Events;
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);
        c.pause();
        let before = env.events().all().len();
        c.unpause();
        assert!(
            env.events().all().len() > before,
            "unpause must emit an event"
        );
    }

    // ── Scoped pause tests ─────────────────────────────────────────────────────

    #[test]
    fn scoped_pause_is_isolated_from_global() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);

        c.pause_scope(PauseScope::Treasury);

        assert!(!c.paused());
        assert!(c.paused_scope(PauseScope::Treasury));
        assert!(!c.paused_scope(PauseScope::Trading));
    }

    #[test]
    fn scoped_unpause_restores_scope() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);

        c.pause_scope(PauseScope::Treasury);
        assert!(c.paused_scope(PauseScope::Treasury));

        c.unpause_scope(PauseScope::Treasury);
        assert!(!c.paused_scope(PauseScope::Treasury));
    }

    #[test]
    fn scoped_guard_blocks_when_paused() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);

        c.pause_scope(PauseScope::Treasury);
        assert!(c.guard_scope(PauseScope::Treasury));
        assert!(!c.guard_scope(PauseScope::Staking));
    }

    #[test]
    fn multiple_scopes_can_be_paused_independently() {
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);

        c.pause_scope(PauseScope::Treasury);
        c.pause_scope(PauseScope::Trading);

        assert!(c.paused_scope(PauseScope::Treasury));
        assert!(c.paused_scope(PauseScope::Trading));
        assert!(c.guard_scope(PauseScope::Treasury));
        assert!(c.guard_scope(PauseScope::Trading));
        assert!(!c.guard_scope(PauseScope::Upgrades));
    }

    #[test]
    fn scoped_pause_emits_event() {
        use soroban_sdk::testutils::Events;
        let (env, id) = setup();
        let c = TestPausableClient::new(&env, &id);

        c.pause_scope(PauseScope::Staking);
        let events = env.events().all();
        assert!(!events.is_empty(), "scoped pause must emit an event");
    }
}
