#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

use crate::{errors::AdminError, param_bounds, SignalRegistry, SignalRegistryClient};

fn setup(env: &Env) -> (Address, Address, SignalRegistryClient<'_>) {
    let admin = Address::generate(env);
    #[allow(deprecated)]
    let id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(env, &id);
    client.initialize(&admin);
    (admin, id, client)
}

// ── Issue 2: Parameter bounds validation ─────────────────────────────────────

#[test]
fn test_set_param_bounds_and_validate_within_range() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let param = Symbol::new(&env, "slippage");
    client.set_param_bounds(&admin, &param, &0i128, &500i128);

    // Within range → Ok
    assert!(client.validate_strategy_param(&param, &250i128).is_ok());
    assert!(client.validate_strategy_param(&param, &0i128).is_ok());
    assert!(client.validate_strategy_param(&param, &500i128).is_ok());
}

#[test]
fn test_validate_param_below_min_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let param = Symbol::new(&env, "interest");
    client.set_param_bounds(&admin, &param, &100i128, &1000i128);

    let result = client.try_validate_strategy_param(&param, &50i128);
    assert_eq!(result, Err(Ok(AdminError::InvalidParameter)));
}

#[test]
fn test_validate_param_above_max_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let param = Symbol::new(&env, "reward");
    client.set_param_bounds(&admin, &param, &1i128, &200i128);

    let result = client.try_validate_strategy_param(&param, &201i128);
    assert_eq!(result, Err(Ok(AdminError::InvalidParameter)));
}

#[test]
fn test_validate_param_no_bounds_declared_always_ok() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let param = Symbol::new(&env, "unknown");
    // No bounds declared → any value passes
    assert!(client.validate_strategy_param(&param, &i128::MAX).is_ok());
    assert!(client.validate_strategy_param(&param, &i128::MIN).is_ok());
}

#[test]
fn test_set_param_bounds_invalid_min_gt_max_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let param = Symbol::new(&env, "bad");
    let result = client.try_set_param_bounds(&admin, &param, &500i128, &100i128);
    assert_eq!(result, Err(Ok(AdminError::InvalidParameter)));
}

#[test]
fn test_set_param_bounds_requires_config_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _id, client) = setup(&env);

    let non_admin = Address::generate(&env);
    let param = Symbol::new(&env, "rate");

    env.set_auths(&[]);
    let result = client.try_set_param_bounds(&non_admin, &param, &0i128, &100i128);
    assert!(result.is_err());
}

#[test]
fn test_get_param_bounds_returns_configured_values() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, id, client) = setup(&env);

    let param = Symbol::new(&env, "multiplier");
    client.set_param_bounds(&admin, &param, &10i128, &500i128);

    env.as_contract(&id, || {
        let bounds = param_bounds::get_param_bounds(&env, param).unwrap();
        assert_eq!(bounds.min, 10i128);
        assert_eq!(bounds.max, 500i128);
    });
}

#[test]
fn test_param_bounds_exact_boundary_values_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let param = Symbol::new(&env, "fee_bps");
    client.set_param_bounds(&admin, &param, &1i128, &100i128);

    // Exact min and max must be accepted
    assert!(client.validate_strategy_param(&param, &1i128).is_ok());
    assert!(client.validate_strategy_param(&param, &100i128).is_ok());
}

#[test]
fn test_param_bounds_can_be_updated() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _id, client) = setup(&env);

    let param = Symbol::new(&env, "limit");
    client.set_param_bounds(&admin, &param, &0i128, &50i128);

    // 60 is out of range
    assert!(client.try_validate_strategy_param(&param, &60i128).is_err());

    // Widen the range
    client.set_param_bounds(&admin, &param, &0i128, &100i128);

    // Now 60 is valid
    assert!(client.validate_strategy_param(&param, &60i128).is_ok());
}
