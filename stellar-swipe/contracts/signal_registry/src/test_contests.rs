#![cfg(test)]
use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::contests::{Contest, ContestEntry, ContestMetric, ContestStatus};
use crate::types::{Signal, SignalAction, SignalStatus};
use soroban_sdk::{testutils::{Address as _, Ledger}, vec, Address, Env, String};

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

#[test]
fn test_create_contest() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let name = String::from_str(&env, "Weekly ROI Contest");
    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60; // 1 week
    let metric = ContestMetric::HighestROI;
    let min_signals = 3;
    let prize_pool = 10000;

    let contest_id = client.create_contest(
        &admin,
        &name,
        &start_time,
        &end_time,
        &metric,
        &min_signals,
        &prize_pool,
    );

    assert_eq!(contest_id, 1);

    let contest = client.get_contest(&contest_id);
    assert_eq!(contest.id, 1);
    assert_eq!(contest.status, ContestStatus::Active);
    assert_eq!(contest.prize_pool, 10000);
}

#[test]
fn test_auto_enter_signal() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let provider = Address::generate(&env);
    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;

    let contest_id = client.create_contest(
        &admin,
        &String::from_str(&env, "Test Contest"),
        &start_time,
        &end_time,
        &ContestMetric::HighestROI,
        &2,
        &5000,
    );

    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100,
        &String::from_str(&env, "Test signal"),
        &(env.ledger().timestamp() + 3600),
        &SignalCategory::SwingTrade,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );

    // Record a trade to update total_roi and total_volume
    client.record_trade_execution(&provider, &signal_id, &100, &250, &1000);

    let contest = client.get_contest(&contest_id);
    let entry = contest.entries.get(provider.clone()).unwrap();

    assert_eq!(entry.signals_submitted.len(), 1);
    assert_eq!(entry.total_roi, 150);
    assert_eq!(entry.total_volume, 1000);
}

#[test]
fn test_finalize_contest_with_winners() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let provider1 = Address::generate(&env);
    let provider2 = Address::generate(&env);
    let provider3 = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 100; // Short contest for testing

    let contest_id = client.create_contest(
        &admin,
        &String::from_str(&env, "ROI Contest"),
        &start_time,
        &end_time,
        &ContestMetric::HighestROI,
        &2,
        &10000,
    );

    // Submit signals for 3 providers with different ROIs
    // Note: Soroban client returns u64 from create_signal, so we don't use .unwrap()
    
    // Provider 1: 2 signals, total ROI 200
    for _ in 0..2 {
        let sid = client.create_signal(
            &provider1,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100,
            &String::from_str(&env, "Test"),
            &(env.ledger().timestamp() + 3600),
            &SignalCategory::SwingTrade,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
        client.record_trade_execution(&provider1, &sid, &10000, &10100, &1000); // 100 bps ROI
    }

    // Provider 2: 3 signals, total ROI 300 (Winner)
    for _ in 0..3 {
        let sid = client.create_signal(
            &provider2,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100,
            &String::from_str(&env, "Test"),
            &(env.ledger().timestamp() + 3600),
            &SignalCategory::SwingTrade,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
        client.record_trade_execution(&provider2, &sid, &10000, &10100, &1000); // 100 bps ROI
    }

    // Provider 3: 2 signals, total ROI 150
    for i in 0..2 {
        let sid = client.create_signal(
            &provider3,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100,
            &String::from_str(&env, "Test"),
            &(env.ledger().timestamp() + 3600),
            &SignalCategory::SwingTrade,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
        let exit = if i == 0 { 10075 } else { 10075 };
        client.record_trade_execution(&provider3, &sid, &10000, &exit, &1000); // 75 bps ROI each
    }

    // Fast forward time to end contest
    env.ledger().set_timestamp(end_time + 1);

    let winners = client.finalize_contest(&contest_id);

    assert_eq!(winners.len(), 3);
    assert_eq!(winners.get(0).unwrap(), provider2); // Highest ROI

    // Check prize distribution
    let prize1 = client.get_provider_prize(&contest_id, &provider2);
    let prize2 = client.get_provider_prize(&contest_id, &provider1);
    let prize3 = client.get_provider_prize(&contest_id, &provider3);

    assert_eq!(prize1, 5000); // 50%
    assert_eq!(prize2, 3000); // 30%
    assert_eq!(prize3, 2000); // 20%
}

#[test]
fn test_contest_min_signals_requirement() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let provider1 = Address::generate(&env);
    let provider2 = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 100;

    let contest_id = client.create_contest(
        &admin,
        &String::from_str(&env, "Min Signals Test"),
        &start_time,
        &end_time,
        &ContestMetric::HighestROI,
        &3, // Require 3 signals minimum
        &5000,
    );

    // Provider1: 2 signals (not qualified)
    for _ in 0..2 {
        let sid = client.create_signal(
            &provider1,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100,
            &String::from_str(&env, "Test"),
            &(env.ledger().timestamp() + 3600),
            &SignalCategory::SwingTrade,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
        client.record_trade_execution(&provider1, &sid, &100, &102, &1000);
    }

    // Provider2: 3 signals (qualified)
    for _ in 2..5 {
        let sid = client.create_signal(
            &provider2,
            &String::from_str(&env, "XLM/USDC"),
            &SignalAction::Buy,
            &100,
            &String::from_str(&env, "Test"),
            &(env.ledger().timestamp() + 3600),
            &SignalCategory::SwingTrade,
            &vec![&env, String::from_str(&env, "test")],
            &RiskLevel::Medium,
        );
        client.record_trade_execution(&provider2, &sid, &100, &101, &1000);
    }

    env.ledger().set_timestamp(end_time + 1);

    let winners = client.finalize_contest(&contest_id);

    // Only provider2 should win (provider1 didn't meet min signals)
    assert_eq!(winners.len(), 1);
    assert_eq!(winners.get(0).unwrap(), provider2);
}

#[test]
fn test_get_contest_leaderboard() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let provider1 = Address::generate(&env);
    let provider2 = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;

    let contest_id = client.create_contest(
        &admin,
        &String::from_str(&env, "Leaderboard Test"),
        &start_time,
        &end_time,
        &ContestMetric::HighestROI,
        &1,
        &5000,
    );

    // Add signals
    let sid1 = client.create_signal(
        &provider1,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100,
        &String::from_str(&env, "Test"),
        &(env.ledger().timestamp() + 3600),
        &SignalCategory::SwingTrade,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );
    client.record_trade_execution(&provider1, &sid1, &100, &103, &1000); // 300 bps ROI

    let sid2 = client.create_signal(
        &provider2,
        &String::from_str(&env, "XLM/USDC"),
        &SignalAction::Buy,
        &100,
        &String::from_str(&env, "Test"),
        &(env.ledger().timestamp() + 3600),
        &SignalCategory::SwingTrade,
        &vec![&env, String::from_str(&env, "test")],
        &RiskLevel::Medium,
    );
    client.record_trade_execution(&provider2, &sid2, &100, &102, &1000); // 200 bps ROI

    let leaderboard = client.get_contest_leaderboard(&contest_id);

    assert_eq!(leaderboard.len(), 2);
    // Provider1 should be first (higher ROI)
    assert_eq!(leaderboard.get(0).unwrap().provider, provider1);
    assert_eq!(leaderboard.get(0).unwrap().score, 300);
    assert_eq!(leaderboard.get(1).unwrap().provider, provider2);
    assert_eq!(leaderboard.get(1).unwrap().score, 200);
}

#[test]
#[should_panic(expected = "ContestNotEnded")]
fn test_finalize_contest_before_end() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;

    let contest_id = client.create_contest(
        &admin,
        &String::from_str(&env, "Early Finalize Test"),
        &start_time,
        &end_time,
        &ContestMetric::HighestROI,
        &1,
        &5000,
    );

    // Try to finalize before end time
    client.finalize_contest(&contest_id);
}
