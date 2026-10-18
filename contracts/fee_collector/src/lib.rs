//! Fee collector contract.
//!
//! Collects protocol fees and exposes a tightly-scoped, auditable path for
//! sweeping negligible dust balances out of the collector.
//!
//! ## Fee collection events (issue #1247)
//!
//! Every successful [`FeeCollector::collect`] call publishes a
//! [`FeesCollected`] event so indexers can reconstruct fee inflows from the
//! event stream without polling storage:
//!
//! * **Topics** — `(Symbol("fees_collected"), asset: Address, source: Address)`
//!   so consumers can filter by fee asset or by fee source.
//! * **Data** — [`FeesCollected`] carrying the `asset`, the `source` that
//!   authorized the collection, the collected `amount`, and the collector
//!   `balance` *after* the collection was applied.
//!
//! The resulting `balance` is a replay checkpoint: replaying
//! `fees_collected.amount` inflows and `dust_swept.amount` outflows in event
//! order reproduces every `balance` value. A failed collection (paused,
//! invalid input, overflow, missing authorization) publishes no event and
//! leaves the balance unchanged. Full consumer documentation lives in
//! `docs/fee_collector_events_and_pause.md`.
//!
//! ## Emergency collection pause (issue #1248)
//!
//! The configured admin can stop new fee intake during an incident with
//! [`FeeCollector::set_collection_paused`]. While paused:
//!
//! * [`FeeCollector::collect`] fails closed with
//!   [`FeeCollectorError::CollectionPaused`] and mutates nothing.
//! * [`FeeCollector::balance`], [`FeeCollector::is_collection_paused`], and
//!   the other read-only queries keep working.
//! * [`FeeCollector::sweep_dust`] stays available: it is admin-only and
//!   bounded below [`DUST_THRESHOLD`], so it is the narrowly authorized
//!   recovery path during an incident.
//! * [`FeeCollector::set_admin`] stays available so a compromised admin key
//!   can still be rotated.
//!
//! Only the admin may pause or resume. A transition to the state that is
//! already active is rejected with [`FeeCollectorError::PauseStateUnchanged`]
//! so every [`PauseChanged`] event marks a real state change.
//!
//! ## Address validation (issue #1171)
//!
//! Every state-changing entrypoint rejects sentinel addresses — the all-zero
//! account ([`ZERO_ACCOUNT_STRKEY`]), the all-zero contract
//! ([`ZERO_CONTRACT_STRKEY`]), and the collector's own address — with
//! [`FeeCollectorError::InvalidAddress`] before any authorization check or
//! storage write. The full inventory is in `docs/address_validation_audit.md`.

#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

/// Balances strictly below this threshold are considered "dust" and are the
/// only balances eligible for the audited sweep path. Any balance at or above
/// this value is an ordinary fee balance and MUST NOT be sweepable.
pub const DUST_THRESHOLD: i128 = 1_000;

/// Issue #1171: strkey of the account whose ed25519 public key is all zero
/// bytes. It has no usable signer and is the conventional "zero address".
pub const ZERO_ACCOUNT_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

/// Issue #1171: strkey of the contract whose 32-byte id is all zero bytes.
pub const ZERO_CONTRACT_STRKEY: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4";

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const BALANCE_KEY: Symbol = symbol_short!("BALANCE");
/// Issue #1247: asset in which the collector accounts its fee balance.
const ASSET_KEY: Symbol = symbol_short!("ASSET");
/// Issue #1248: emergency collection pause flag.
const PAUSED_KEY: Symbol = symbol_short!("PAUSED");

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FeeCollectorError {
    NotInitialized = 1,
    Unauthorized = 2,
    InvalidDestination = 3,
    InvalidAmount = 4,
    AmountExceedsBalance = 5,
    AmountNotDust = 6,
    /// Issue #1248: `initialize` was called on an already initialized
    /// collector. Re-initialization would let any caller replace the admin
    /// and bypass the pause controls.
    AlreadyInitialized = 7,
    /// Issue #1248: fee collection is rejected while the emergency pause is
    /// active. No state was mutated.
    CollectionPaused = 8,
    /// Issue #1248: the requested pause state is already active.
    PauseStateUnchanged = 9,
    /// Issue #1247: the collected amount cannot be added to the balance
    /// without overflowing. No state was mutated.
    ArithmeticOverflow = 10,
    /// Issue #1171: an address argument is a sentinel (all-zero account,
    /// all-zero contract, or this contract). Shared code across contracts.
    InvalidAddress = 100,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DustSwept {
    pub caller: Address,
    pub destination: Address,
    pub amount: i128,
}

/// Auditable record of a material contract configuration change.
///
/// Emitted whenever a configuration setting is updated so indexers and
/// operators can discover the change, the prior value, the new value, and the
/// actor that authorized it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigChanged {
    pub setting: Symbol,
    pub old_value: Address,
    pub new_value: Address,
    pub actor: Address,
}

/// Structured payload for a successful fee collection (issue #1247).
///
/// `balance` is the collector balance after `amount` was applied, so a
/// consumer can verify its replayed balance at every event.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeesCollected {
    pub asset: Address,
    pub source: Address,
    pub amount: i128,
    pub balance: i128,
}

/// Auditable event emitted when the emergency collection pause changes
/// (issue #1248).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseChanged {
    pub paused: bool,
    pub actor: Address,
}

#[contract]
pub struct FeeCollector;

#[contractimpl]
impl FeeCollector {
    /// Initialize the collector with an admin authority and the fee asset it
    /// accounts for.
    ///
    /// Can only be called once; a second call fails with
    /// [`FeeCollectorError::AlreadyInitialized`].
    pub fn initialize(env: Env, admin: Address, fee_asset: Address) -> Result<(), FeeCollectorError> {
        Self::require_valid_address(&env, &admin)?;
        Self::require_valid_address(&env, &fee_asset)?;
        if env.storage().instance().has(&ADMIN_KEY) {
            return Err(FeeCollectorError::AlreadyInitialized);
        }
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&ASSET_KEY, &fee_asset);
        env.storage().instance().set(&BALANCE_KEY, &0i128);
        env.storage().instance().set(&PAUSED_KEY, &false);
        Ok(())
    }

    /// Update the collector's admin authority.
    ///
    /// Only the current admin may rotate the admin role. The change is recorded
    /// in a [`ConfigChanged`] event capturing the setting, prior admin, new
    /// admin, and the authorizing actor.
    pub fn set_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), FeeCollectorError> {
        Self::require_valid_address(&env, &caller)?;
        Self::require_valid_address(&env, &new_admin)?;
        caller.require_auth();

        let admin = Self::require_admin(&env, &caller)?;

        env.storage().instance().set(&ADMIN_KEY, &new_admin);

        env.events().publish(
            (Symbol::new(&env, "config_changed"),),
            ConfigChanged {
                setting: ADMIN_KEY,
                old_value: admin,
                new_value: new_admin,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Record `amount` of collected fees from `source` into the collector
    /// balance and return the resulting balance.
    ///
    /// `source` must authorize the call so the attribution in the emitted
    /// [`FeesCollected`] event cannot be spoofed. The call fails without
    /// mutating state or emitting an event when collection is paused, the
    /// amount is not strictly positive, or the new balance would overflow.
    pub fn collect(env: Env, source: Address, amount: i128) -> Result<i128, FeeCollectorError> {
        Self::require_valid_address(&env, &source)?;
        source.require_auth();

        let asset: Address = env
            .storage()
            .instance()
            .get(&ASSET_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;

        // Issue #1248: fail closed while the emergency pause is active.
        if Self::is_collection_paused(env.clone()) {
            return Err(FeeCollectorError::CollectionPaused);
        }

        if amount <= 0 {
            return Err(FeeCollectorError::InvalidAmount);
        }

        let balance: i128 = env.storage().instance().get(&BALANCE_KEY).unwrap_or(0);
        let new_balance = balance
            .checked_add(amount)
            .ok_or(FeeCollectorError::ArithmeticOverflow)?;
        env.storage().instance().set(&BALANCE_KEY, &new_balance);

        // Issue #1247: publish the inflow with the resulting balance so the
        // collector balance can be replayed from events alone.
        env.events().publish(
            (Symbol::new(&env, "fees_collected"), asset.clone(), source.clone()),
            FeesCollected {
                asset,
                source,
                amount,
                balance: new_balance,
            },
        );

        Ok(new_balance)
    }

    /// Current collector balance. Available while collection is paused.
    pub fn balance(env: Env) -> i128 {
        env.storage().instance().get(&BALANCE_KEY).unwrap_or(0)
    }

    /// Asset in which the collector balance is denominated (issue #1247).
    pub fn fee_asset(env: Env) -> Result<Address, FeeCollectorError> {
        env.storage()
            .instance()
            .get(&ASSET_KEY)
            .ok_or(FeeCollectorError::NotInitialized)
    }

    /// Issue #1248: pause or resume new fee collection.
    ///
    /// Only the configured admin may change the pause state, and only to the
    /// state that is not already active. Each successful transition emits a
    /// [`PauseChanged`] event attributed to the admin.
    pub fn set_collection_paused(
        env: Env,
        caller: Address,
        paused: bool,
    ) -> Result<(), FeeCollectorError> {
        Self::require_valid_address(&env, &caller)?;
        caller.require_auth();
        Self::require_admin(&env, &caller)?;

        if Self::is_collection_paused(env.clone()) == paused {
            return Err(FeeCollectorError::PauseStateUnchanged);
        }
        env.storage().instance().set(&PAUSED_KEY, &paused);

        env.events().publish(
            (Symbol::new(&env, "pause_changed"),),
            PauseChanged {
                paused,
                actor: caller,
            },
        );
        Ok(())
    }

    /// Issue #1248: report whether new fee collection is currently paused.
    pub fn is_collection_paused(env: Env) -> bool {
        env.storage().instance().get(&PAUSED_KEY).unwrap_or(false)
    }

    /// Sweep a negligible dust balance to an explicit destination.
    ///
    /// Only the configured admin may call this, the amount must be strictly
    /// below [`DUST_THRESHOLD`], and the destination must be a non-sentinel
    /// address other than the caller. The destination, amount, and
    /// authorizing caller are recorded in a [`DustSwept`] event for audit
    /// purposes. Remains available while collection is paused (issue #1248).
    pub fn sweep_dust(
        env: Env,
        caller: Address,
        destination: Address,
        amount: i128,
    ) -> Result<(), FeeCollectorError> {
        Self::require_valid_address(&env, &caller)?;
        Self::require_valid_address(&env, &destination)?;
        caller.require_auth();

        Self::require_admin(&env, &caller)?;

        if destination == caller {
            return Err(FeeCollectorError::InvalidDestination);
        }

        if amount <= 0 {
            return Err(FeeCollectorError::InvalidAmount);
        }

        if amount >= DUST_THRESHOLD {
            return Err(FeeCollectorError::AmountNotDust);
        }

        let balance: i128 = env.storage().instance().get(&BALANCE_KEY).unwrap_or(0);
        if amount > balance {
            return Err(FeeCollectorError::AmountExceedsBalance);
        }

        env.storage().instance().set(&BALANCE_KEY, &(balance - amount));

        env.events().publish(
            (Symbol::new(&env, "dust_swept"),),
            DustSwept {
                caller,
                destination,
                amount,
            },
        );

        Ok(())
    }

    /// Load the configured admin and require that `caller` is that admin.
    fn require_admin(env: &Env, caller: &Address) -> Result<Address, FeeCollectorError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if *caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }
        Ok(admin)
    }

    /// Issue #1171: reject sentinel addresses with a stable error.
    fn require_valid_address(env: &Env, address: &Address) -> Result<(), FeeCollectorError> {
        if *address == env.current_contract_address()
            || *address == Address::from_str(env, ZERO_ACCOUNT_STRKEY)
            || *address == Address::from_str(env, ZERO_CONTRACT_STRKEY)
        {
            return Err(FeeCollectorError::InvalidAddress);
        }
        Ok(())
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke};
    use soroban_sdk::{IntoVal, TryFromVal, Val, Vec};

    fn setup(env: &Env) -> (FeeCollectorClient<'_>, Address, Address) {
        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(env, &contract_id);
        let admin = Address::generate(env);
        let asset = Address::generate(env);
        client.initialize(&admin, &asset);
        (client, admin, asset)
    }

    fn sentinels(env: &Env, client: &FeeCollectorClient) -> [Address; 3] {
        [
            Address::from_str(env, ZERO_ACCOUNT_STRKEY),
            Address::from_str(env, ZERO_CONTRACT_STRKEY),
            client.address.clone(),
        ]
    }

    fn fees_collected_events(env: &Env) -> Vec<FeesCollected> {
        let mut out = Vec::new(env);
        for (_, _, data) in env.events().all().iter() {
            if let Ok(event) = FeesCollected::try_from_val(env, &data) {
                out.push_back(event);
            }
        }
        out
    }

    #[test]
    fn sweeps_dust_below_threshold() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let destination = Address::generate(&env);
        let source = Address::generate(&env);

        client.collect(&source, &(DUST_THRESHOLD - 1));
        client.sweep_dust(&admin, &destination, &(DUST_THRESHOLD - 1));

        assert_eq!(client.balance(), 0);
    }

    #[test]
    fn rejects_ordinary_fee_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let destination = Address::generate(&env);
        let source = Address::generate(&env);

        client.collect(&source, &DUST_THRESHOLD);
        let result = client.try_sweep_dust(&admin, &destination, &DUST_THRESHOLD);

        assert_eq!(result, Err(Ok(FeeCollectorError::AmountNotDust)));
        assert_eq!(client.balance(), DUST_THRESHOLD);
    }

    #[test]
    fn rejects_unauthorized_caller() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _asset) = setup(&env);
        let attacker = Address::generate(&env);
        let destination = Address::generate(&env);
        let source = Address::generate(&env);

        client.collect(&source, &(DUST_THRESHOLD - 1));
        let result = client.try_sweep_dust(&attacker, &destination, &(DUST_THRESHOLD - 1));

        assert_eq!(result, Err(Ok(FeeCollectorError::Unauthorized)));
    }

    #[test]
    fn rejects_invalid_amounts() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let destination = Address::generate(&env);
        let source = Address::generate(&env);

        client.collect(&source, &(DUST_THRESHOLD - 1));
        assert_eq!(
            client.try_sweep_dust(&admin, &destination, &0),
            Err(Ok(FeeCollectorError::InvalidAmount))
        );
        assert_eq!(
            client.try_sweep_dust(&admin, &destination, &DUST_THRESHOLD),
            Err(Ok(FeeCollectorError::AmountNotDust))
        );
    }

    #[test]
    fn emits_config_changed_on_admin_update() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let new_admin = Address::generate(&env);

        client.set_admin(&admin, &new_admin);

        let events = env.events().all();
        let expected = ConfigChanged {
            setting: ADMIN_KEY,
            old_value: admin.clone(),
            new_value: new_admin.clone(),
            actor: admin.clone(),
        };
        assert!(
            events.iter().any(|(_, _, data)| {
                ConfigChanged::try_from_val(&env, &data).map_or(false, |e| e == expected)
            }),
            "expected a ConfigChanged event for the admin setting"
        );
    }

    #[test]
    fn emits_no_config_changed_on_rejected_update() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, _asset) = setup(&env);
        let attacker = Address::generate(&env);
        let new_admin = Address::generate(&env);

        let result = client.try_set_admin(&attacker, &new_admin);
        assert_eq!(result, Err(Ok(FeeCollectorError::Unauthorized)));

        let events = env.events().all();
        assert!(
            !events.iter().any(|(_, _, data)| {
                ConfigChanged::try_from_val(&env, &data).is_ok()
            }),
            "no ConfigChanged event should be emitted for a rejected update"
        );
    }

    #[test]
    fn rejects_reinitialization() {
        // Issue #1248: a second initialize must not replace the admin.
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, asset) = setup(&env);
        let attacker = Address::generate(&env);

        assert_eq!(
            client.try_initialize(&attacker, &asset),
            Err(Ok(FeeCollectorError::AlreadyInitialized))
        );
        assert_eq!(
            client.try_set_collection_paused(&attacker, &true),
            Err(Ok(FeeCollectorError::Unauthorized))
        );
    }

    // ── Issue #1247: fee collection events ──────────────────────────────────

    #[test]
    fn collect_emits_structured_event_with_resulting_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, asset) = setup(&env);
        let source = Address::generate(&env);

        assert_eq!(client.collect(&source, &250), 250);

        // The event shape is part of the consumer contract: topics carry the
        // event name, asset, and source; the data carries the full payload.
        let events = env.events().all();
        let (contract, topics, data) = events.last().unwrap();
        assert_eq!(contract, client.address);
        let expected_topics: Vec<Val> =
            (Symbol::new(&env, "fees_collected"), asset.clone(), source.clone()).into_val(&env);
        assert_eq!(topics, expected_topics);
        assert_eq!(
            FeesCollected::try_from_val(&env, &data).unwrap(),
            FeesCollected {
                asset: asset.clone(),
                source: source.clone(),
                amount: 250,
                balance: 250,
            }
        );
    }

    #[test]
    fn collection_events_replay_to_the_stored_balance() {
        // Replaying inflows and dust outflows reproduces every checkpoint.
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let first = Address::generate(&env);
        let second = Address::generate(&env);
        let destination = Address::generate(&env);

        let mut replayed: i128 = 0;
        for (source, amount) in [(&first, 400i128), (&second, 350), (&first, 5)] {
            client.collect(source, &amount);
            let events = fees_collected_events(&env);
            assert_eq!(events.len(), 1);
            let event = events.get(0).unwrap();
            assert_eq!(event.source, source.clone());
            replayed += event.amount;
            assert_eq!(event.balance, replayed);
        }

        client.sweep_dust(&admin, &destination, &5);
        let swept = env
            .events()
            .all()
            .iter()
            .find_map(|(_, _, data)| DustSwept::try_from_val(&env, &data).ok())
            .unwrap();
        replayed -= swept.amount;
        assert_eq!(replayed, client.balance());
    }

    #[test]
    fn failed_collections_emit_no_event_and_keep_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let source = Address::generate(&env);
        client.collect(&source, &100);

        assert_eq!(
            client.try_collect(&source, &0),
            Err(Ok(FeeCollectorError::InvalidAmount))
        );
        assert!(fees_collected_events(&env).is_empty());
        assert_eq!(
            client.try_collect(&source, &-5),
            Err(Ok(FeeCollectorError::InvalidAmount))
        );
        assert!(fees_collected_events(&env).is_empty());
        assert_eq!(
            client.try_collect(&source, &i128::MAX),
            Err(Ok(FeeCollectorError::ArithmeticOverflow))
        );
        assert!(fees_collected_events(&env).is_empty());

        client.set_collection_paused(&admin, &true);
        assert_eq!(
            client.try_collect(&source, &10),
            Err(Ok(FeeCollectorError::CollectionPaused))
        );
        assert!(fees_collected_events(&env).is_empty());

        assert_eq!(client.balance(), 100);
    }

    #[test]
    fn collect_requires_source_authorization() {
        let env = Env::default();
        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let asset = Address::generate(&env);
        client.initialize(&admin, &asset);
        let source = Address::generate(&env);

        // No authorization supplied for `source`.
        assert!(client.try_collect(&source, &100).is_err());
        assert!(fees_collected_events(&env).is_empty());
        assert_eq!(client.balance(), 0);
    }

    #[test]
    fn collect_before_initialize_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(&env, &contract_id);
        let source = Address::generate(&env);

        assert_eq!(
            client.try_collect(&source, &100),
            Err(Ok(FeeCollectorError::NotInitialized))
        );
    }

    // ── Issue #1248: emergency collection pause ────────────────────────────

    #[test]
    fn pause_blocks_collection_but_keeps_queries_and_dust_recovery() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset) = setup(&env);
        let source = Address::generate(&env);
        let destination = Address::generate(&env);
        client.collect(&source, &(DUST_THRESHOLD + 10));
        assert!(!client.is_collection_paused());

        client.set_collection_paused(&admin, &true);
        assert!(client.is_collection_paused());

        assert_eq!(
            client.try_collect(&source, &1),
            Err(Ok(FeeCollectorError::CollectionPaused))
        );
        // Read-only queries are unaffected.
        assert_eq!(client.balance(), DUST_THRESHOLD + 10);
        assert_eq!(client.fee_asset(), asset);
        // Approved dust recovery remains available and keeps its own bounds.
        client.sweep_dust(&admin, &destination, &10);
        assert_eq!(client.balance(), DUST_THRESHOLD);
        assert_eq!(
            client.try_sweep_dust(&admin, &destination, &DUST_THRESHOLD),
            Err(Ok(FeeCollectorError::AmountNotDust))
        );

        client.set_collection_paused(&admin, &false);
        assert!(!client.is_collection_paused());
        assert_eq!(client.collect(&source, &1), DUST_THRESHOLD + 1);
    }

    #[test]
    fn pause_transitions_emit_auditable_events() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);

        client.set_collection_paused(&admin, &true);
        let (_, topics, data) = env.events().all().last().unwrap();
        let expected_topics: Vec<Val> = (Symbol::new(&env, "pause_changed"),).into_val(&env);
        assert_eq!(topics, expected_topics);
        assert_eq!(
            PauseChanged::try_from_val(&env, &data).unwrap(),
            PauseChanged {
                paused: true,
                actor: admin.clone(),
            }
        );

        client.set_collection_paused(&admin, &false);
        let (_, _, data) = env.events().all().last().unwrap();
        assert_eq!(
            PauseChanged::try_from_val(&env, &data).unwrap(),
            PauseChanged {
                paused: false,
                actor: admin.clone(),
            }
        );
    }

    #[test]
    fn redundant_pause_transition_is_rejected_without_event() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);

        assert_eq!(
            client.try_set_collection_paused(&admin, &false),
            Err(Ok(FeeCollectorError::PauseStateUnchanged))
        );
        client.set_collection_paused(&admin, &true);
        assert_eq!(
            client.try_set_collection_paused(&admin, &true),
            Err(Ok(FeeCollectorError::PauseStateUnchanged))
        );
        assert!(!env
            .events()
            .all()
            .iter()
            .any(|(_, _, data)| PauseChanged::try_from_val(&env, &data).is_ok()));
        assert!(client.is_collection_paused());
    }

    #[test]
    fn non_admin_cannot_pause_or_resume() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let attacker = Address::generate(&env);

        assert_eq!(
            client.try_set_collection_paused(&attacker, &true),
            Err(Ok(FeeCollectorError::Unauthorized))
        );
        assert!(!client.is_collection_paused());

        client.set_collection_paused(&admin, &true);
        assert_eq!(
            client.try_set_collection_paused(&attacker, &false),
            Err(Ok(FeeCollectorError::Unauthorized))
        );
        assert!(client.is_collection_paused());
    }

    #[test]
    fn pause_requires_admin_signature() {
        // Naming the admin as `caller` is not enough: the admin must sign.
        let env = Env::default();
        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let asset = Address::generate(&env);
        client.initialize(&admin, &asset);

        assert!(client.try_set_collection_paused(&admin, &true).is_err());
        assert!(!client.is_collection_paused());

        client
            .mock_auths(&[MockAuth {
                address: &admin,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "set_collection_paused",
                    args: (admin.clone(), true).into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .set_collection_paused(&admin, &true);
        assert!(client.is_collection_paused());
    }

    #[test]
    fn rotated_admin_controls_pause() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let new_admin = Address::generate(&env);

        client.set_collection_paused(&admin, &true);
        // Admin rotation stays available during an incident.
        client.set_admin(&admin, &new_admin);
        assert_eq!(
            client.try_set_collection_paused(&admin, &false),
            Err(Ok(FeeCollectorError::Unauthorized))
        );
        client.set_collection_paused(&new_admin, &false);
        assert!(!client.is_collection_paused());
    }

    // ── Issue #1171: address validation ────────────────────────────────────

    #[test]
    fn initialize_rejects_sentinel_addresses() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(&env, &contract_id);
        let valid = Address::generate(&env);

        for sentinel in sentinels(&env, &client) {
            assert_eq!(
                client.try_initialize(&sentinel, &valid),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
            assert_eq!(
                client.try_initialize(&valid, &sentinel),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
        }
        // Nothing was stored by the rejected calls.
        assert_eq!(
            client.try_fee_asset(),
            Err(Ok(FeeCollectorError::NotInitialized))
        );
    }

    #[test]
    fn state_changing_entrypoints_reject_sentinel_addresses() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, _asset) = setup(&env);
        let destination = Address::generate(&env);
        let source = Address::generate(&env);
        client.collect(&source, &10);

        for sentinel in sentinels(&env, &client) {
            assert_eq!(
                client.try_set_admin(&sentinel, &destination),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
            assert_eq!(
                client.try_set_admin(&admin, &sentinel),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
            assert_eq!(
                client.try_collect(&sentinel, &10),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
            assert_eq!(
                client.try_set_collection_paused(&sentinel, &true),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
            assert_eq!(
                client.try_sweep_dust(&sentinel, &destination, &1),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
            assert_eq!(
                client.try_sweep_dust(&admin, &sentinel, &1),
                Err(Ok(FeeCollectorError::InvalidAddress))
            );
        }

        // No rejected call changed state.
        assert_eq!(client.balance(), 10);
        assert!(!client.is_collection_paused());
        client.set_admin(&admin, &destination);
    }

    #[test]
    fn sentinel_strkeys_are_well_formed() {
        let env = Env::default();
        let account = Address::from_str(&env, ZERO_ACCOUNT_STRKEY);
        let contract = Address::from_str(&env, ZERO_CONTRACT_STRKEY);
        assert_ne!(account, contract);
        assert_ne!(contract, Address::generate(&env));
    }
}
