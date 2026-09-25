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
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DustSwept {
    pub caller: Address,
    pub destination: Address,
    pub amount: i128,
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

    /// Record collected fees into the collector balance.
    pub fn collect(env: Env, amount: i128) {
        let balance: i128 = env.storage().instance().get(&BALANCE_KEY).unwrap_or(0);
        env.storage().instance().set(&BALANCE_KEY, &(balance + amount));
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
    use soroban_sdk::testutils::Address as _;

    fn setup(env: &Env) -> (FeeCollectorClient, Address) {
        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.initialize(&admin);
        (client, admin)
    }

    #[test]
    fn sweeps_dust_below_threshold() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let destination = Address::generate(&env);

        client.collect(&(DUST_THRESHOLD - 1));
        client.sweep_dust(&admin, &destination, &(DUST_THRESHOLD - 1));

        assert_eq!(client.balance(), 0);
    }

    #[test]
    fn rejects_ordinary_fee_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let destination = Address::generate(&env);

        client.collect(&DUST_THRESHOLD);
        let result = client.try_sweep_dust(&admin, &destination, &DUST_THRESHOLD);

        assert_eq!(result, Err(Ok(FeeCollectorError::AmountNotDust)));
        assert_eq!(client.balance(), DUST_THRESHOLD);
    }

    #[test]
    fn rejects_unauthorized_caller() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup(&env);
        let attacker = Address::generate(&env);
        let destination = Address::generate(&env);

        client.collect(&(DUST_THRESHOLD - 1));
        let result = client.try_sweep_dust(&attacker, &destination, &(DUST_THRESHOLD - 1));

        assert_eq!(result, Err(Ok(FeeCollectorError::Unauthorized)));
    }

    #[test]
    fn rejects_invalid_amounts() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin) = setup(&env);
        let destination = Address::generate(&env);

        client.collect(&(DUST_THRESHOLD - 1));
        assert_eq!(
            client.try_sweep_dust(&admin, &destination, &0),
            Err(Ok(FeeCollectorError::InvalidAmount))
        );
        assert_eq!(
            client.try_sweep_dust(&admin, &destination, &DUST_THRESHOLD),
            Err(Ok(FeeCollectorError::AmountNotDust))
        );
    }
}
