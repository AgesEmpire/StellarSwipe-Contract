//! Tests for admin/keeper-gated pruning of expired signals (Issue #779).

#![cfg(test)]

use super::*;
use crate::types::SignalStatus;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Map, String,
};

const NOW: u64 = 100_000;

fn make_signal(env: &Env, id: u64, provider: &Address, expiry: u64) -> Signal {
    Signal {
        id,
        provider: provider.clone(),
        asset_pair: String::from_str(env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100_000,
        rationale: String::from_str(env, "Test signal"),
        timestamp: env.ledger().timestamp(),
        expiry,
        status: SignalStatus::Active,
        executions: 0,
        successful_executions: 0,
        total_volume: 0,
        total_roi: 0,
        category: SignalCategory::SWING,
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
        tags: Vec::new(env),
        submitted_at: env.ledger().timestamp(),
        rationale_hash: String::from_str(env, "Test signal"),
        confidence: 50,
        adoption_count: 0,
        ai_validation_score: None,
        avg_copier_roi_bps: 0,
        copier_closed_count: 0,
        warning_emitted: false,
        benchmark_return_bps: None,
        alpha_bps: None,
    }
}

/// Seed the signals map (and per-category index) with `expired_count` signals
/// past expiry followed by `active_count` live ones. Ids start at 1.
fn seed_signals(env: &Env, contract_id: &Address, expired_count: u64, active_count: u64) {
    let provider = Address::generate(env);
    env.as_contract(contract_id, || {
        let mut map = Map::new(env);
        let mut ids = Vec::new(env);
        for id in 1..=expired_count {
            map.set(id, make_signal(env, id, &provider, NOW - 1_000));
            ids.push_back(id);
        }
        for id in (expired_count + 1)..=(expired_count + active_count) {
            map.set(id, make_signal(env, id, &provider, NOW + 1_000));
            ids.push_back(id);
        }
        env.storage().instance().set(&StorageKey::Signals, &map);

        let mut cat_map: Map<SignalCategory, Vec<u64>> = Map::new(env);
        cat_map.set(SignalCategory::SWING, ids);
        env.storage()
            .instance()
            .set(&StorageKey::ActiveSignalsByCategory, &cat_map);
    });
}

fn signals_len(env: &Env, contract_id: &Address) -> u32 {
    env.as_contract(contract_id, || {
        let map: Map<u64, Signal> = env
            .storage()
            .instance()
            .get(&StorageKey::Signals)
            .unwrap_or(Map::new(env));
        map.len()
    })
}

fn setup() -> (Env, Address, SignalRegistryClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, contract_id, client, admin)
}

#[test]
fn prune_with_no_expired_signals_returns_zero() {
    let (env, contract_id, client, admin) = setup();
    seed_signals(&env, &contract_id, 0, 3);

    assert_eq!(client.prune_expired_signals(&admin, &10), 0);
    assert_eq!(signals_len(&env, &contract_id), 3);
}

#[test]
fn prune_zero_max_entries_is_noop() {
    let (env, contract_id, client, admin) = setup();
    seed_signals(&env, &contract_id, 4, 1);

    assert_eq!(client.prune_expired_signals(&admin, &0), 0);
    assert_eq!(signals_len(&env, &contract_id), 5);
}

#[test]
fn prune_fewer_expired_than_max_entries() {
    let (env, contract_id, client, admin) = setup();
    seed_signals(&env, &contract_id, 2, 3);

    assert_eq!(client.prune_expired_signals(&admin, &10), 2);
    assert_eq!(signals_len(&env, &contract_id), 3);

    // Only the live signals remain
    let active = client.get_active_signals(&0, &10, &SortOption::RecencyDesc, &None, &None);
    assert_eq!(active.len(), 3);
}

#[test]
fn prune_more_expired_than_max_entries_partial() {
    let (env, contract_id, client, admin) = setup();
    seed_signals(&env, &contract_id, 5, 2);

    // First pass prunes only up to the budget
    assert_eq!(client.prune_expired_signals(&admin, &2), 2);
    assert_eq!(signals_len(&env, &contract_id), 5);

    // Second pass drains the rest
    assert_eq!(client.prune_expired_signals(&admin, &10), 3);
    assert_eq!(signals_len(&env, &contract_id), 2);
}

#[test]
fn prune_unauthorized_caller_rejected() {
    let (env, contract_id, client, _admin) = setup();
    seed_signals(&env, &contract_id, 2, 0);

    let stranger = Address::generate(&env);
    let result = client.try_prune_expired_signals(&stranger, &10);
    assert_eq!(result, Err(Ok(AdminError::Unauthorized)));
    assert_eq!(signals_len(&env, &contract_id), 2);
}

#[test]
fn prune_allowlisted_keeper_may_call() {
    let (env, contract_id, client, admin) = setup();
    seed_signals(&env, &contract_id, 3, 1);

    let keeper = Address::generate(&env);
    client.add_prune_keeper(&admin, &keeper);
    assert_eq!(client.get_prune_keepers().len(), 1);

    assert_eq!(client.prune_expired_signals(&keeper, &10), 3);
    assert_eq!(signals_len(&env, &contract_id), 1);

    // Removed keepers lose access again
    client.remove_prune_keeper(&admin, &keeper);
    let result = client.try_prune_expired_signals(&keeper, &10);
    assert_eq!(result, Err(Ok(AdminError::Unauthorized)));
}

#[test]
fn only_admin_may_manage_keepers() {
    let (env, _contract_id, client, _admin) = setup();

    let stranger = Address::generate(&env);
    let keeper = Address::generate(&env);
    assert_eq!(
        client.try_add_prune_keeper(&stranger, &keeper),
        Err(Ok(AdminError::Unauthorized))
    );
    assert_eq!(
        client.try_remove_prune_keeper(&stranger, &keeper),
        Err(Ok(AdminError::Unauthorized))
    );
}

#[test]
fn prune_updates_category_index() {
    let (env, contract_id, client, admin) = setup();
    seed_signals(&env, &contract_id, 2, 1);

    client.prune_expired_signals(&admin, &10);

    env.as_contract(&contract_id, || {
        let cat_map: Map<SignalCategory, Vec<u64>> = env
            .storage()
            .instance()
            .get(&StorageKey::ActiveSignalsByCategory)
            .unwrap();
        let ids = cat_map.get(SignalCategory::SWING).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids.get(0).unwrap(), 3); // only the live signal's id remains
    });
}

#[test]
fn health_check_reports_expired_signal_count() {
    let (env, contract_id, client, admin) = setup();
    seed_signals(&env, &contract_id, 2, 1);

    let h = client.health_check();
    assert!(h.is_initialized);
    assert_eq!(h.expired_signal_count, 2);

    client.prune_expired_signals(&admin, &10);
    let h = client.health_check();
    assert_eq!(h.expired_signal_count, 0);
}
