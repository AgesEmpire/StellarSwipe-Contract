#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, testutils::Ledger, Env, String, Vec, vec};

use crate::combos::{
    ComboStatus, ComboType, Condition, ConditionType, ComponentSignal, ConditionGate,
};
use crate::categories::{SignalCategory, RiskLevel};


// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn setup<'a>(env: &'a Env) -> (Address, SignalRegistryClient<'a>) {
    env.mock_all_auths();
    env.ledger().set_timestamp(10_000);

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    (admin, client)
}

fn make_signal(env: &Env, client: &SignalRegistryClient, provider: &Address) -> u64 {
    let expiry = env.ledger().timestamp() + 3600;
    client.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(env, "Bullish"),
        &expiry,
        &SignalCategory::SWING,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Medium,
    )
}

fn make_sell_signal(env: &Env, client: &SignalRegistryClient, provider: &Address) -> u64 {
    let expiry = env.ledger().timestamp() + 3600;
    client.create_signal(
        provider,
        &String::from_str(env, "BTC/USDC"),
        &SignalAction::Sell,
        &4_500_000,
        &String::from_str(env, "Bearish BTC"),
        &expiry,
        &SignalCategory::SCALP,
        &vec![env, String::from_str(env, "crypto")],
        &RiskLevel::High,
    )
}

fn two_component_50_50(env: &Env, sig1: u64, sig2: u64) -> Vec<ComponentSignal> {
    let mut comps = Vec::new(env);
    comps.push_back(ComponentSignal {
        signal_id: sig1,
        weight: 5000,
        condition: ConditionGate::None,
    });
    comps.push_back(ComponentSignal {
        signal_id: sig2,
        weight: 5000,
        condition: ConditionGate::None,
    });
    comps
}

// -----------------------------------------------------------------------
// create_combo_signal
// -----------------------------------------------------------------------

#[test]
fn test_create_simultaneous_combo() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_sell_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "BTC-XLM Pairs Trade"),
            &comps,
            &ComboType::Simultaneous,
        );

    assert_eq!(combo_id, 1);

    let combo = client.get_combo_signal(&combo_id).unwrap();
    assert_eq!(combo.id, combo_id);
    assert!(matches!(combo.status, ComboStatus::Active));
    assert_eq!(combo.component_signals.len(), 2);
}

#[test]
fn test_create_sequential_combo() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 3333, condition: ConditionGate::None });
    comps.push_back(ComponentSignal { signal_id: sig2, weight: 3333, condition: ConditionGate::None });
    // 3334 to make sum exactly 10000
    let sig3 = make_signal(&env, &client, &provider);
    comps.push_back(ComponentSignal { signal_id: sig3, weight: 3334, condition: ConditionGate::None });

    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Ladder Entry"),
            &comps,
            &ComboType::Sequential,
        );

    let combo = client.get_combo_signal(&combo_id).unwrap();
    assert_eq!(combo.component_signals.len(), 3);
    assert!(matches!(combo.combo_type, ComboType::Sequential));
}

#[test]
fn test_create_conditional_combo() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal {
        signal_id: sig1,
        weight: 5000,
        condition: ConditionGate::None, // first signal has no condition
    });
    comps.push_back(ComponentSignal {
        signal_id: sig2,
        weight: 5000,
        condition: ConditionGate::Some(Condition {
            depends_on: sig1,
            condition_type: ConditionType::Success,
        }),
    });

    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Buy XLM + conditional follow-up"),
            &comps,
            &ComboType::Conditional,
        );

    assert!(combo_id > 0);
}

#[test]
fn test_create_combo_invalid_weights_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 4000, condition: ConditionGate::None });
    comps.push_back(ComponentSignal { signal_id: sig2, weight: 4000, condition: ConditionGate::None });

    comps.push_back(ComponentSignal {
        signal_id: sig1,
        weight: 4000,
        condition: ConditionGate::None,
    });
    comps.push_back(ComponentSignal {
        signal_id: sig2,
        weight: 4000,
        condition: ConditionGate::None,
    });
    // total = 8000, not 10000

    let result = client.try_create_combo_signal(
        &provider,
        &String::from_str(&env, "Bad weights"),
        &comps,
        &ComboType::Simultaneous,
    );
    assert!(result.is_err());
}

#[test]
fn test_create_combo_not_signal_owner_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let attacker = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);

    // attacker tries to create combo using provider's signals
    let result = client.try_create_combo_signal(
        &attacker,
        &String::from_str(&env, "Hijack"),
        &comps,
        &ComboType::Simultaneous,
    );
    assert!(result.is_err());
}

#[test]
fn test_create_combo_signal_not_found_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 5000, condition: ConditionGate::None });
    comps.push_back(ComponentSignal { signal_id: 999, weight: 5000, condition: ConditionGate::None }); // non-existent

    let result = client.try_create_combo_signal(
        &provider,
        &String::from_str(&env, "Bad signal ref"),
        &comps,
        &ComboType::Simultaneous,
    );
    assert!(result.is_err());
}

#[test]
fn test_create_combo_no_components_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let comps: Vec<ComponentSignal> = Vec::new(&env);

    let result = client.try_create_combo_signal(
        &provider,
        &String::from_str(&env, "Empty"),
        &comps,
        &ComboType::Simultaneous,
    );
    assert!(result.is_err());
}

#[test]
fn test_create_combo_invalid_condition_reference_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 5000, condition: ConditionGate::None });
    comps.push_back(ComponentSignal {
        signal_id: sig2,
        weight: 5000,
        condition: ConditionGate::Some(Condition {
            depends_on: 999, // references a signal NOT in the combo
            condition_type: ConditionType::Success,
        }),
    });

    let result = client.try_create_combo_signal(
        &provider,
        &String::from_str(&env, "Bad condition ref"),
        &comps,
        &ComboType::Conditional,
    );
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// execute_combo_signal
// -----------------------------------------------------------------------

#[test]
fn test_execute_simultaneous_combo() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_sell_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Pairs Trade"),
            &comps,
            &ComboType::Simultaneous,
        );

    let executions = client
        .execute_combo_signal(&combo_id, &user, &1_000_000);

    assert_eq!(executions.len(), 2);

    // Both should be executed (not skipped)
    assert!(!executions.get(0).unwrap().skipped);
    assert!(!executions.get(1).unwrap().skipped);

    // Each should get half the capital
    assert_eq!(executions.get(0).unwrap().amount, 500_000);
    assert_eq!(executions.get(1).unwrap().amount, 500_000);
}

#[test]
fn test_execute_sequential_combo() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);
    let sig3 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 3333, condition: ConditionGate::None });
    comps.push_back(ComponentSignal { signal_id: sig2, weight: 3333, condition: ConditionGate::None });
    comps.push_back(ComponentSignal { signal_id: sig3, weight: 3334, condition: ConditionGate::None });

    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Ladder"),
            &comps,
            &ComboType::Sequential,
        );

    let executions = client
        .execute_combo_signal(&combo_id, &user, &3_000_000);

    assert_eq!(executions.len(), 3);
    for i in 0..3u32 {
        assert!(!executions.get(i).unwrap().skipped);
    }
}

#[test]
fn test_execute_conditional_combo_condition_met() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 5000, condition: ConditionGate::None });
    comps.push_back(ComponentSignal {
        signal_id: sig2,
        weight: 5000,
        condition: ConditionGate::Some(Condition {
            depends_on: sig1,
            condition_type: ConditionType::Success,
        }),
    });

    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Conditional Success"),
            &comps,
            &ComboType::Conditional,
        );

    // Record trade after combo references signals so they stay Active at creation time.
    client.record_trade_execution(&user, &sig1, &100_000, &110_000, &1_000_000);

    let executions = client
        .execute_combo_signal(&combo_id, &user, &1_000_000);

    // sig1 executes and has positive ROI from simulate_trade_roi
    // sig2 condition (Success on sig1) is evaluated against sig1's execution in this combo
    assert_eq!(executions.len(), 2);
    // sig1 always executes
    assert!(!executions.get(0).unwrap().skipped);
    // sig2 depends on sig1's ROI in THIS combo execution
    // since sig1 has historical positive ROI, simulate_trade_roi returns > 0
    // so sig2 executes too
    assert!(!executions.get(1).unwrap().skipped);
}

#[test]
fn test_execute_conditional_combo_condition_not_met() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    // sig1 has NO prior executions → simulate_trade_roi returns 0 (not > 0)
    // Condition: Success (roi > 0) → NOT met

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 5000, condition: ConditionGate::None });
    comps.push_back(ComponentSignal {
        signal_id: sig2,
        weight: 5000,
        condition: ConditionGate::Some(Condition {
            depends_on: sig1,
            condition_type: ConditionType::Success,
        }),
    });

    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Conditional Fail"),
            &comps,
            &ComboType::Conditional,
        );

    let executions = client
        .execute_combo_signal(&combo_id, &user, &1_000_000);

    assert_eq!(executions.len(), 2);
    assert!(!executions.get(0).unwrap().skipped); // sig1 always runs
    assert!(executions.get(1).unwrap().skipped); // sig2 skipped — condition not met
}

#[test]
fn test_execute_conditional_roi_above_threshold() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let mut comps = Vec::new(&env);
    comps.push_back(ComponentSignal { signal_id: sig1, weight: 5000, condition: ConditionGate::None });
    comps.push_back(ComponentSignal {
        signal_id: sig2,
        weight: 5000,
        condition: ConditionGate::Some(Condition {
            depends_on: sig1,
            condition_type: ConditionType::RoiAbove(200), // threshold 200 bps
        }),
    });

    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "ROI Threshold"),
            &comps,
            &ComboType::Conditional,
        );

    client.record_trade_execution(&user, &sig1, &100_000, &105_000, &1_000_000);

    let executions = client
        .execute_combo_signal(&combo_id, &user, &1_000_000);

    // sig1's ROI (500 bps) > threshold (200 bps) → sig2 executes
    assert!(!executions.get(1).unwrap().skipped);
}

#[test]
fn test_execute_combo_expired_signal_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Will expire"),
            &comps,
            &ComboType::Simultaneous,
        );

    // Advance past signal expiry
    env.ledger().set_timestamp(10_000 + 7200);

    let result = client.try_execute_combo_signal(&combo_id, &user, &1_000_000);
    assert!(result.is_err());
}

#[test]
fn test_execute_cancelled_combo_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_sell_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "To cancel"),
            &comps,
            &ComboType::Simultaneous,
        );

    client.cancel_combo_signal(&combo_id, &provider);

    let result = client.try_execute_combo_signal(&combo_id, &user, &1_000_000);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// cancel_combo_signal
// -----------------------------------------------------------------------

#[test]
fn test_cancel_combo_success() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Cancel me"),
            &comps,
            &ComboType::Simultaneous,
        );

    client.cancel_combo_signal(&combo_id, &provider);

    let combo = client.get_combo_signal(&combo_id).unwrap();
    assert!(matches!(combo.status, ComboStatus::Cancelled));
}

#[test]
fn test_cancel_combo_wrong_provider_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let attacker = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Mine"),
            &comps,
            &ComboType::Simultaneous,
        );

    let result = client.try_cancel_combo_signal(&combo_id, &attacker);
    assert!(result.is_err());
}

#[test]
fn test_cancel_combo_twice_fails() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Double cancel"),
            &comps,
            &ComboType::Simultaneous,
        );

    client.cancel_combo_signal(&combo_id, &provider);
    let result = client.try_cancel_combo_signal(&combo_id, &provider);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// Performance tracking
// -----------------------------------------------------------------------

#[test]
fn test_combo_performance_no_executions() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Perf test"),
            &comps,
            &ComboType::Simultaneous,
        );

    let perf = client.get_combo_performance(&combo_id).unwrap();
    assert_eq!(perf.total_executions, 0);
    assert_eq!(perf.combined_roi, 0);
    assert_eq!(perf.total_volume, 0);
}

#[test]
fn test_combo_performance_after_execution() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Perf after exec"),
            &comps,
            &ComboType::Simultaneous,
        );

    client
        .execute_combo_signal(&combo_id, &user, &2_000_000);

    let perf = client.get_combo_performance(&combo_id).unwrap();
    assert_eq!(perf.total_executions, 1);
    assert_eq!(perf.total_volume, 2_000_000);
}

#[test]
fn test_combo_execution_history_recorded() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    let sig1 = make_signal(&env, &client, &provider);
    let sig2 = make_signal(&env, &client, &provider);

    let comps = two_component_50_50(&env, sig1, sig2);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "History test"),
            &comps,
            &ComboType::Simultaneous,
        );

    client.execute_combo_signal(&combo_id, &user, &1_000_000);
    client.execute_combo_signal(&combo_id, &user, &2_000_000);
    let history = client.get_combo_executions(&combo_id);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0).unwrap().total_amount, 1_000_000);
    assert_eq!(history.get(1).unwrap().total_amount, 2_000_000);
}

// -----------------------------------------------------------------------
// Full end-to-end workflow
// -----------------------------------------------------------------------

#[test]
fn test_full_pairs_trade_workflow() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let provider = Address::generate(&env);
    let user = Address::generate(&env);

    // Create a pairs trade: long XLM, short BTC (50/50)
    let xlm_signal = make_signal(&env, &client, &provider); // BUY XLM/USDC
    let btc_signal = make_sell_signal(&env, &client, &provider); // SELL BTC/USDC

    let comps = two_component_50_50(&env, xlm_signal, btc_signal);
    let combo_id = client
        .create_combo_signal(
            &provider,
            &String::from_str(&env, "Long XLM / Short BTC"),
            &comps,
            &ComboType::Simultaneous,
        );

    // Execute combo with 10,000 USDC
    let executions = client
        .execute_combo_signal(&combo_id, &user, &10_000_000);

    assert_eq!(executions.len(), 2);
    assert_eq!(executions.get(0).unwrap().amount, 5_000_000); // 50%
    assert_eq!(executions.get(1).unwrap().amount, 5_000_000); // 50%

    // Verify performance was recorded
    let perf = client.get_combo_performance(&combo_id).unwrap();
    assert_eq!(perf.total_executions, 1);
    assert_eq!(perf.total_volume, 10_000_000);

    // Combo remains active for further executions
    let combo = client.get_combo_signal(&combo_id).unwrap();
    assert!(matches!(combo.status, ComboStatus::Active));
}
