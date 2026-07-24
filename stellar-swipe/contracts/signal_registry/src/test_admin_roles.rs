#![cfg(test)]

use crate::{AdminError, AdminRole, SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use stellar_swipe_common::emergency::{CircuitBreakerConfig, CAT_TRADING};

fn setup() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    #[allow(deprecated)]
    let id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &id);
    let root_admin = Address::generate(&env);
    client.initialize(&root_admin);
    (env, root_admin, client)
}

fn circuit_config() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        volume_spike_mult: 10,
        max_failure_rate_bps: 5_000,
        max_price_move_bps: 1_000,
        max_loss_1h: 100_000_000,
    }
}

#[test]
fn root_admin_assigns_and_reads_scoped_admin_roles() {
    let (env, root_admin, client) = setup();
    let config_admin = Address::generate(&env);
    let emergency_admin = Address::generate(&env);
    let treasury_admin = Address::generate(&env);

    client.set_admin_role(&root_admin, &AdminRole::Config, &config_admin);
    client.set_admin_role(&root_admin, &AdminRole::Emergency, &emergency_admin);
    client.set_admin_role(&root_admin, &AdminRole::Treasury, &treasury_admin);

    assert_eq!(
        client.get_admin_role(&AdminRole::Config).unwrap(),
        config_admin
    );
    assert_eq!(
        client.get_admin_role(&AdminRole::Emergency).unwrap(),
        emergency_admin
    );
    assert_eq!(
        client.get_admin_role(&AdminRole::Treasury).unwrap(),
        treasury_admin
    );
}

#[test]
fn config_admin_can_update_config_but_not_emergency_or_treasury() {
    let (env, root_admin, client) = setup();
    let config_admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    client.set_admin_role(&root_admin, &AdminRole::Config, &config_admin);

    client.set_min_stake(&config_admin, &250_000_000);
    client.set_tier_signal_limits(&config_admin, &2, &4, &8);
    assert_eq!(client.get_config().min_stake, 250_000_000);
    assert_eq!(client.get_config().gold_signal_limit, 8);

    assert_eq!(
        client.try_pause_trading(&config_admin),
        Err(Ok(AdminError::Unauthorized))
    );
    assert_eq!(
        client.try_set_platform_treasury(&config_admin, &treasury),
        Err(Ok(AdminError::Unauthorized))
    );
}

#[test]
fn emergency_admin_can_pause_and_tune_breakers_but_not_config() {
    let (env, root_admin, client) = setup();
    let emergency_admin = Address::generate(&env);
    client.set_admin_role(&root_admin, &AdminRole::Emergency, &emergency_admin);

    client.pause_category(
        &emergency_admin,
        &soroban_sdk::String::from_str(&env, CAT_TRADING),
        &Some(300),
        &soroban_sdk::String::from_str(&env, "scoped emergency pause"),
    );
    assert!(client.is_paused());
    client.set_circuit_breaker_config(&emergency_admin, &circuit_config());

    assert_eq!(
        client.try_set_min_stake(&emergency_admin, &250_000_000),
        Err(Ok(AdminError::Unauthorized))
    );
}

#[test]
fn treasury_admin_can_update_treasury_but_not_config_or_emergency() {
    let (env, root_admin, client) = setup();
    let treasury_admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    client.set_admin_role(&root_admin, &AdminRole::Treasury, &treasury_admin);

    client.set_platform_treasury(&treasury_admin, &treasury);
    assert_eq!(client.get_platform_treasury().unwrap(), treasury);

    assert_eq!(
        client.try_set_trade_fee(&treasury_admin, &20),
        Err(Ok(AdminError::Unauthorized))
    );
    assert_eq!(
        client.try_pause_trading(&treasury_admin),
        Err(Ok(AdminError::Unauthorized))
    );
}
