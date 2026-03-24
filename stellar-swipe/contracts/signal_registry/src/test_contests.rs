#![cfg(test)]
use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::contests::{Contest, ContestEntry, ContestMetric, ContestStatus};
use crate::types::{Signal, SignalAction, SignalStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

#[test]
fn test_create_contest() {
    let env = Env::default();
    env.mock_all_auths();

    let name = String::from_str(&env, "Weekly ROI Contest");
    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60; // 1 week
    let metric = ContestMetric::HighestROI;
    let min_signals = 3;
    let prize_pool = 10000;

    let contest_id = contests::create_contest(
        &env,
        name,
        start_time,
        end_time,
        metric,
        min_signals,
        prize_pool,
    )
    .unwrap();

    assert_eq!(contest_id, 1);

    let contest = contests::get_contest(&env, contest_id).unwrap();
    assert_eq!(contest.id, 1);
    assert_eq!(contest.status, ContestStatus::Active);
    assert_eq!(contest.prize_pool, 10000);
}

#[test]
fn test_auto_enter_signal() {
    let env = Env::default();
    env.mock_all_auths();

    let provider = Address::generate(&env);
    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;

    let contest_id = contests::create_contest(
        &env,
        String::from_str(&env, "Test Contest"),
        start_time,
        end_time,
        ContestMetric::HighestROI,
        2,
        5000,
    )
    .unwrap();

    let signal = Signal {
        id: 1,
        provider: provider.clone(),
        asset_pair: String::from_str(&env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100,
        rationale: String::from_str(&env, "Test signal"),
        timestamp: env.ledger().timestamp(),
        expiry: env.ledger().timestamp() + 3600,
        status: SignalStatus::Active,
        executions: 1,
        successful_executions: 1,
        total_volume: 1000,
        total_roi: 150,
        category: SignalCategory::SwingTrade,
        tags: soroban_sdk::Vec::new(&env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
    };

    contests::auto_enter_signal(&env, &signal).unwrap();

    let contest = contests::get_contest(&env, contest_id).unwrap();
    let entry = contest.entries.get(provider.clone()).unwrap();

    assert_eq!(entry.signals_submitted.len(), 1);
    assert_eq!(entry.total_roi, 150);
    assert_eq!(entry.total_volume, 1000);
}

#[test]
fn test_finalize_contest_with_winners() {
    let env = Env::default();
    env.mock_all_auths();

    let provider1 = Address::generate(&env);
    let provider2 = Address::generate(&env);
    let provider3 = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 100; // Short contest for testing

    let contest_id = contests::create_contest(
        &env,
        String::from_str(&env, "ROI Contest"),
        start_time,
        end_time,
        ContestMetric::HighestROI,
        2,
        10000,
    )
    .unwrap();

    // Submit signals for 3 providers with different ROIs
    let signals = vec![
        (provider1.clone(), 200, 2), // ROI: 200, 2 signals
        (provider2.clone(), 300, 3), // ROI: 300, 3 signals (winner)
        (provider3.clone(), 150, 2), // ROI: 150, 2 signals
    ];

    for (provider, roi, count) in signals {
        for i in 0..count {
            let signal = Signal {
                id: i as u64,
                provider: provider.clone(),
                asset_pair: String::from_str(&env, "XLM/USDC"),
                action: SignalAction::Buy,
                price: 100,
                rationale: String::from_str(&env, "Test"),
                timestamp: env.ledger().timestamp(),
                expiry: env.ledger().timestamp() + 3600,
                status: SignalStatus::Active,
                executions: 1,
                successful_executions: 1,
                total_volume: 1000,
                total_roi: roi / count,
                category: SignalCategory::SwingTrade,
                tags: soroban_sdk::Vec::new(&env),
                risk_level: RiskLevel::Medium,
                is_collaborative: false,
            };
            contests::auto_enter_signal(&env, &signal).unwrap();
        }
    }

    // Fast forward time to end contest
    env.ledger().with_mut(|li| li.timestamp = end_time + 1);

    let winners = contests::finalize_contest(&env, contest_id).unwrap();

    assert_eq!(winners.len(), 3);
    assert_eq!(winners.get(0).unwrap(), provider2); // Highest ROI

    // Check prize distribution
    let prize1 = contests::get_provider_prize(&env, contest_id, provider2.clone());
    let prize2 = contests::get_provider_prize(&env, contest_id, provider1.clone());
    let prize3 = contests::get_provider_prize(&env, contest_id, provider3.clone());

    assert_eq!(prize1, 5000); // 50%
    assert_eq!(prize2, 3000); // 30%
    assert_eq!(prize3, 2000); // 20%
}

#[test]
fn test_contest_min_signals_requirement() {
    let env = Env::default();
    env.mock_all_auths();

    let provider1 = Address::generate(&env);
    let provider2 = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 100;

    let contest_id = contests::create_contest(
        &env,
        String::from_str(&env, "Min Signals Test"),
        start_time,
        end_time,
        ContestMetric::HighestROI,
        3, // Require 3 signals minimum
        5000,
    )
    .unwrap();

    // Provider1: 2 signals (not qualified)
    for i in 0..2 {
        let signal = Signal {
            id: i,
            provider: provider1.clone(),
            asset_pair: String::from_str(&env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100,
            rationale: String::from_str(&env, "Test"),
            timestamp: env.ledger().timestamp(),
            expiry: env.ledger().timestamp() + 3600,
            status: SignalStatus::Active,
            executions: 1,
            successful_executions: 1,
            total_volume: 1000,
            total_roi: 200,
            category: SignalCategory::SwingTrade,
            tags: soroban_sdk::Vec::new(&env),
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
        };
        contests::auto_enter_signal(&env, &signal).unwrap();
    }

    // Provider2: 3 signals (qualified)
    for i in 2..5 {
        let signal = Signal {
            id: i,
            provider: provider2.clone(),
            asset_pair: String::from_str(&env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100,
            rationale: String::from_str(&env, "Test"),
            timestamp: env.ledger().timestamp(),
            expiry: env.ledger().timestamp() + 3600,
            status: SignalStatus::Active,
            executions: 1,
            successful_executions: 1,
            total_volume: 1000,
            total_roi: 150,
            category: SignalCategory::SwingTrade,
            tags: soroban_sdk::Vec::new(&env),
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
        };
        contests::auto_enter_signal(&env, &signal).unwrap();
    }

    env.ledger().with_mut(|li| li.timestamp = end_time + 1);

    let winners = contests::finalize_contest(&env, contest_id).unwrap();

    // Only provider2 should win (provider1 didn't meet min signals)
    assert_eq!(winners.len(), 1);
    assert_eq!(winners.get(0).unwrap(), provider2);
}

#[test]
fn test_get_contest_leaderboard() {
    let env = Env::default();
    env.mock_all_auths();

    let provider1 = Address::generate(&env);
    let provider2 = Address::generate(&env);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;

    let contest_id = contests::create_contest(
        &env,
        String::from_str(&env, "Leaderboard Test"),
        start_time,
        end_time,
        ContestMetric::HighestROI,
        1,
        5000,
    )
    .unwrap();

    // Add signals
    let signal1 = Signal {
        id: 1,
        provider: provider1.clone(),
        asset_pair: String::from_str(&env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100,
        rationale: String::from_str(&env, "Test"),
        timestamp: env.ledger().timestamp(),
        expiry: env.ledger().timestamp() + 3600,
        status: SignalStatus::Active,
        executions: 1,
        successful_executions: 1,
        total_volume: 1000,
        total_roi: 300,
        category: SignalCategory::SwingTrade,
        tags: soroban_sdk::Vec::new(&env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
    };

    let signal2 = Signal {
        id: 2,
        provider: provider2.clone(),
        asset_pair: String::from_str(&env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100,
        rationale: String::from_str(&env, "Test"),
        timestamp: env.ledger().timestamp(),
        expiry: env.ledger().timestamp() + 3600,
        status: SignalStatus::Active,
        executions: 1,
        successful_executions: 1,
        total_volume: 1000,
        total_roi: 200,
        category: SignalCategory::SwingTrade,
        tags: soroban_sdk::Vec::new(&env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
    };

    contests::auto_enter_signal(&env, &signal1).unwrap();
    contests::auto_enter_signal(&env, &signal2).unwrap();

    let leaderboard = contests::get_contest_leaderboard(&env, contest_id).unwrap();

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
    env.mock_all_auths();

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 7 * 24 * 60 * 60;

    let contest_id = contests::create_contest(
        &env,
        String::from_str(&env, "Early Finalize Test"),
        start_time,
        end_time,
        ContestMetric::HighestROI,
        1,
        5000,
    )
    .unwrap();

    // Try to finalize before end time
    contests::finalize_contest(&env, contest_id).unwrap();
}
