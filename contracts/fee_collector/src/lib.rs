//! Fee collector contract.
//!
//! Collects protocol fees and exposes a tightly-scoped, auditable path for
//! sweeping negligible dust balances out of the collector.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

/// Balances strictly below this threshold are considered "dust" and are the
/// only balances eligible for the audited sweep path. Any balance at or above
/// this value is an ordinary fee balance and MUST NOT be sweepable.
pub const DUST_THRESHOLD: i128 = 1_000;

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const BALANCE_KEY: Symbol = symbol_short!("BALANCE");
const SOURCE_KEY: Symbol = symbol_short!("SOURCE");

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
    UnknownRevenueSource = 7,
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

/// Auditable record of a revenue source allowlist change.
///
/// Emitted whenever a revenue source is added to or removed from the
/// allowlist so indexers and operators can attribute deposits and discover
/// which sources are authorized to deposit fees.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevenueSourceChanged {
    pub source: Address,
    pub allowed: bool,
    pub actor: Address,
}

#[contract]
pub struct FeeCollector;

#[contractimpl]
impl FeeCollector {
    /// Initialize the collector with an admin authority.
    pub fn initialize(env: Env, admin: Address) {
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&BALANCE_KEY, &0i128);
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
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        env.storage().instance().set(&ADMIN_KEY, &new_admin);

        env.events().publish(
            (symbol_short!("config_changed"),),
            ConfigChanged {
                setting: ADMIN_KEY,
                old_value: admin,
                new_value: new_admin,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Authorize a revenue source to deposit fees into the collector.
    ///
    /// Only the configured admin may add a source. The change is recorded in a
    /// [`RevenueSourceChanged`] event so the allowlist is fully auditable.
    pub fn add_revenue_source(
        env: Env,
        caller: Address,
        source: Address,
    ) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        env.storage()
            .persistent()
            .set(&(SOURCE_KEY, source.clone()), &true);

        env.events().publish(
            (symbol_short!("rev_source"),),
            RevenueSourceChanged {
                source,
                allowed: true,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Revoke a revenue source's authorization to deposit fees.
    ///
    /// Only the configured admin may remove a source. Removal takes effect
    /// immediately, so a source removed during an in-flight accounting period
    /// can no longer deposit. The change is recorded in a
    /// [`RevenueSourceChanged`] event.
    pub fn remove_revenue_source(
        env: Env,
        caller: Address,
        source: Address,
    ) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        env.storage()
            .persistent()
            .remove(&(SOURCE_KEY, source.clone()));

        env.events().publish(
            (symbol_short!("rev_source"),),
            RevenueSourceChanged {
                source,
                allowed: false,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Whether a revenue source is currently authorized to deposit fees.
    pub fn is_revenue_source(env: Env, source: Address) -> bool {
        env.storage()
            .persistent()
            .get(&(SOURCE_KEY, source))
            .unwrap_or(false)
    }

    /// Record collected fees into the collector balance.
    ///
    /// Only allowlisted revenue sources may deposit. The authorization check
    /// runs before any balance mutation so deposits from unknown sources fail
    /// before any transfer or accounting side effect is attempted.
    pub fn collect(
        env: Env,
        source: Address,
        amount: i128,
    ) -> Result<(), FeeCollectorError> {
        source.require_auth();

        let allowed: bool = env
            .storage()
            .persistent()
            .get(&(SOURCE_KEY, source.clone()))
            .unwrap_or(false);
        if !allowed {
            return Err(FeeCollectorError::UnknownRevenueSource);
        }

        let balance: i128 = env.storage().instance().get(&BALANCE_KEY).unwrap_or(0);
        env.storage().instance().set(&BALANCE_KEY, &(balance + amount));

        Ok(())
    }

    /// Current collector balance.
    pub fn balance(env: Env) -> i128 {
        env.storage().instance().get(&BALANCE_KEY).unwrap_or(0)
    }

    /// Sweep a negligible dust balance to an explicit destination.
    ///
    /// Only the configured admin may call this, the amount must be strictly
    /// below [`DUST_THRESHOLD`], and the destination must be non-zero. The
    /// destination, amount, and authorizing caller are recorded in a
    /// [`DustSwept`] event for audit purposes.
    pub fn sweep_dust(
        env: Env,
        caller: Address,
        destination: Address,
        amount: i128,
    ) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

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
            (symbol_short!("dust_swept"),),
            DustSwept {
                caller,
                destination,
                amount,
            },
        );

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _};

    fn setup(env: &Env) -> (FeeCollectorClient, Address) {
        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.initialize(&admin);
        (client, admin)
    }

    fn register_source(env: &Env, client: &FeeCollectorClient, admin: &Address) -> Address {
        let source = Address::generate(env);
        client.add_revenue_source(admin, &source);
        source
    }

    #[test]
    fn sweeps_dust_below_threshold() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let destination = Address::generate(&env);
        let source = register_source(&env, &client, &admin);

        client.collect(&source, &(DUST_THRESHOLD - 1));
        client.sweep_dust(&admin, &destination, &(DUST_THRESHOLD - 1));

        assert_eq!(client.balance(), 0);
    }

    #[test]
    fn rejects_ordinary_fee_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let destination = Address::generate(&env);
        let source = register_source(&env, &client, &admin);

        client.collect(&source, &DUST_THRESHOLD);
        let result = client.try_sweep_dust(&admin, &destination, &DUST_THRESHOLD);

        assert_eq!(result, Err(Ok(FeeCollectorError::AmountNotDust)));
        assert_eq!(client.balance(), DUST_THRESHOLD);
    }

    #[test]
    fn rejects_unauthorized_caller() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let attacker = Address::generate(&env);
        let destination = Address::generate(&env);
        let source = register_source(&env, &client, &admin);

        client.collect(&source, &(DUST_THRESHOLD - 1));
        let result = client.try_sweep_dust(&attacker, &destination, &(DUST_THRESHOLD - 1));

        assert_eq!(result, Err(Ok(FeeCollectorError::Unauthorized)));
    }

    #[test]
    fn rejects_invalid_amounts() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let destination = Address::generate(&env);
        let source = register_source(&env, &client, &admin);

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
        let (client, admin) = setup(&env);
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
        let (client, _admin) = setup(&env);
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
    fn add_revenue_source_requires_admin_and_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let source = Address::generate(&env);

        client.add_revenue_source(&admin, &source);
        assert!(client.is_revenue_source(&source));

        let events = env.events().all();
        let expected = RevenueSourceChanged {
            source: source.clone(),
            allowed: true,
            actor: admin.clone(),
        };
        assert!(
            events.iter().any(|(_, _, data)| {
                RevenueSourceChanged::try_from_val(&env, &data).map_or(false, |e| e == expected)
            }),
            "expected a RevenueSourceChanged event when adding a source"
        );
    }

    #[test]
    fn add_revenue_source_rejects_unauthorized_caller() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup(&env);
        let attacker = Address::generate(&env);
        let source = Address::generate(&env);

        let result = client.try_add_revenue_source(&attacker, &source);
        assert_eq!(result, Err(Ok(FeeCollectorError::Unauthorized)));
        assert!(!client.is_revenue_source(&source));
    }

    #[test]
    fn remove_revenue_source_requires_admin_and_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let source = register_source(&env, &client, &admin);

        client.remove_revenue_source(&admin, &source);
        assert!(!client.is_revenue_source(&source));

        let events = env.events().all();
        let expected = RevenueSourceChanged {
            source: source.clone(),
            allowed: false,
            actor: admin.clone(),
        };
        assert!(
            events.iter().any(|(_, _, data)| {
                RevenueSourceChanged::try_from_val(&env, &data).map_or(false, |e| e == expected)
            }),
            "expected a RevenueSourceChanged event when removing a source"
        );
    }

    #[test]
    fn remove_revenue_source_rejects_unauthorized_caller() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let attacker = Address::generate(&env);
        let source = register_source(&env, &client, &admin);

        let result = client.try_remove_revenue_source(&attacker, &source);
        assert_eq!(result, Err(Ok(FeeCollectorError::Unauthorized)));
        assert!(client.is_revenue_source(&source));
    }

    #[test]
    fn rejects_deposit_from_unknown_source() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup(&env);
        let unknown = Address::generate(&env);

        let result = client.try_collect(&unknown, &100);
        assert_eq!(result, Err(Ok(FeeCollectorError::UnknownRevenueSource)));
        assert_eq!(client.balance(), 0);
    }

    #[test]
    fn rejects_deposit_after_source_removed_mid_period() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let source = register_source(&env, &client, &admin);

        // Deposit during the accounting period.
        client.collect(&source, &500);
        assert_eq!(client.balance(), 500);

        // Source is removed while the period is still in flight.
        client.remove_revenue_source(&admin, &source);

        // Further deposits from the removed source must fail before transfer.
        let result = client.try_collect(&source, &500);
        assert_eq!(result, Err(Ok(FeeCollectorError::UnknownRevenueSource)));
        assert_eq!(client.balance(), 500);
    }
}
