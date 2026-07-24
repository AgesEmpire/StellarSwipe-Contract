/// Shared pausable module (Issue #561).
///
/// Provides a consistent pause-state storage key, pause/unpause helpers, a
/// "reject if paused" guard, and a uniform event emitted on every state
/// change.  Any contract that needs an emergency-pause capability imports
/// these helpers instead of rolling its own.
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
use soroban_sdk::{contracttype, symbol_short, Env, Symbol};

/// Storage key for the pause flag.  Defined as a distinct enum so it cannot
/// collide with contract-local key enums whose variants have different
/// discriminants.
#[contracttype]
#[derive(Clone)]
pub enum PausableKey {
    Paused,
}

/// Returns `true` when the contract is paused.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<_, bool>(&PausableKey::Paused)
        .unwrap_or(false)
}

/// Persists the new pause state and emits a consistent event.
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

/// Returns `Err(true)` (a sentinel) if the contract is currently paused,
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
    }

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(TestPausable, ());
        (env, id)
    }

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
}
