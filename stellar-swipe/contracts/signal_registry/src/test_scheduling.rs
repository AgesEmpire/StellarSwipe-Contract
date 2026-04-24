#![cfg(test)]

use crate::types::{RecurrencePattern, SignalAction, SignalData};
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

#[test]
fn test_schedule_and_publish() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let provider = Address::generate(&env);

    let signal_data = SignalData {
        asset_pair: String::from_str(&env, "BTC/USD"),
        action: SignalAction::Buy,
        price: 50000_0000000,
        rationale: String::from_str(&env, "Strong support level bounce"),
    };

    let current_time = env.ledger().timestamp();
    let publish_at = current_time + 60; // 1 minute in future

    let recurrence = RecurrencePattern {
        is_recurring: false,
        interval_seconds: 0,
        repeat_count: 0,
    };

    // 1. Schedule
    let schedule_id = client.schedule(&provider, &signal_data, &publish_at, &recurrence);
    assert_eq!(schedule_id, 0);

    // 2. Fast forward time
    env.ledger().set_timestamp(publish_at + 1);

    // 3. Publish
    let published_ids = client.trigger_scheduled_publications();
    assert_eq!(published_ids.len(), 1);
    assert_eq!(published_ids.get(0).unwrap(), 0);
}

#[test]
fn test_cancel_schedule() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let provider = Address::generate(&env);

    let signal_data = SignalData {
        asset_pair: String::from_str(&env, "ETH/USD"),
        action: SignalAction::Sell,
        price: 3000_0000000,
        rationale: String::from_str(&env, "Bearish divergence"),
    };

    let current_time = env.ledger().timestamp();
    let publish_at = current_time + 3600;

    let recurrence = RecurrencePattern {
        is_recurring: false,
        interval_seconds: 0,
        repeat_count: 0,
    };

    let schedule_id = client.schedule(&provider, &signal_data, &publish_at, &recurrence);

    // Cancel
    client.cancel_schedule(&provider, &schedule_id);

    // Fast forward and attempt publish
    env.ledger().set_timestamp(publish_at + 1);
    let published_ids = client.trigger_scheduled_publications();

    assert_eq!(published_ids.len(), 0);
}
