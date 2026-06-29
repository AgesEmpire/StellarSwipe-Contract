#![cfg(test)]

use crate::categories::{RiskLevel, SignalCategory};
use crate::types::SignalAction;
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

fn setup(env: &Env) -> (Address, SignalRegistryClient) {
    env.mock_all_auths();
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (admin, client)
}

fn add_signal(env: &Env, client: &SignalRegistryClient, provider: &Address, category: SignalCategory) -> u64 {
    client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "categorization test"),
        &(env.ledger().timestamp() + 86_400),
        &category,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Medium,
    )
}

#[test]
fn empty_category_returns_empty_vec() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let result = client.list_signals_by_category(&SignalCategory::SCALP, &0, &10);
    assert_eq!(result.len(), 0);
}

#[test]
fn signals_returned_for_correct_category() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let p = Address::generate(&env);
    add_signal(&env, &client, &p, SignalCategory::SWING);
    add_signal(&env, &client, &p, SignalCategory::SCALP);
    add_signal(&env, &client, &p, SignalCategory::SWING);

    let swings = client.list_signals_by_category(&SignalCategory::SWING, &0, &50);
    assert_eq!(swings.len(), 2);

    let scalps = client.list_signals_by_category(&SignalCategory::SCALP, &0, &50);
    assert_eq!(scalps.len(), 1);
}

#[test]
fn signals_from_other_categories_excluded() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let p = Address::generate(&env);
    add_signal(&env, &client, &p, SignalCategory::LONG_TERM);
    add_signal(&env, &client, &p, SignalCategory::ARBITRAGE);

    let result = client.list_signals_by_category(&SignalCategory::SWING, &0, &50);
    assert_eq!(result.len(), 0);
}

#[test]
fn pagination_offset_skips_signals() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let p = Address::generate(&env);
    let id1 = add_signal(&env, &client, &p, SignalCategory::PREMIUM);
    let id2 = add_signal(&env, &client, &p, SignalCategory::PREMIUM);
    let id3 = add_signal(&env, &client, &p, SignalCategory::PREMIUM);

    let page1 = client.list_signals_by_category(&SignalCategory::PREMIUM, &0, &2);
    assert_eq!(page1.len(), 2);

    let page2 = client.list_signals_by_category(&SignalCategory::PREMIUM, &2, &2);
    assert_eq!(page2.len(), 1);

    // Offset beyond count returns empty
    let page3 = client.list_signals_by_category(&SignalCategory::PREMIUM, &5, &10);
    assert_eq!(page3.len(), 0);

    // All three IDs present across pages (no duplicates)
    let ids: soroban_sdk::Vec<u64> = {
        let mut v = soroban_sdk::Vec::new(&env);
        for i in 0..page1.len() { v.push_back(page1.get(i).unwrap().id); }
        for i in 0..page2.len() { v.push_back(page2.get(i).unwrap().id); }
        v
    };
    assert!(ids.contains(id1));
    assert!(ids.contains(id2));
    assert!(ids.contains(id3));
    let _ = (id1, id2, id3);
}

#[test]
fn limit_clamped_to_50() {
    let env = Env::default();
    let (_, client) = setup(&env);
    // Default bronze limit = 5 per provider; use 12 providers × 5 = 60 signals
    for _ in 0..12 {
        let p = Address::generate(&env);
        for _ in 0..5 {
            add_signal(&env, &client, &p, SignalCategory::SCALP);
        }
    }
    // limit param 200 is clamped to 50 by the entrypoint
    let result = client.list_signals_by_category(&SignalCategory::SCALP, &0, &200);
    assert_eq!(result.len(), 50);
}

#[test]
fn returned_signals_all_active_and_not_expired() {
    let env = Env::default();
    let (_, client) = setup(&env);
    let p = Address::generate(&env);
    add_signal(&env, &client, &p, SignalCategory::SWING);

    let result = client.list_signals_by_category(&SignalCategory::SWING, &0, &10);
    for i in 0..result.len() {
        let sig = result.get(i).unwrap();
        assert!(sig.expiry > env.ledger().timestamp());
    }
}
