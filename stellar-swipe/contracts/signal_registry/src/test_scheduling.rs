#![cfg(test)]

use crate::categories::RiskLevel;
use crate::scheduling::{get_scheduled_signal_data, ScheduleDataKey};
use crate::types::{
    RecurrencePattern, ScheduleStatus, ScheduledSignal, SignalAction, SignalDataV1, SignalDataV2,
    VersionedSignalData,
};
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

fn make_v2_signal(env: &Env) -> SignalDataV2 {
    SignalDataV2 {
        asset_pair: String::from_str(env, "BTC/USD"),
        action: SignalAction::Buy,
        price: 50000_0000000,
        rationale: String::from_str(env, "Strong support level bounce"),
        confidence: 80,
        risk_level: RiskLevel::Medium,
    }
}

#[test]
fn test_schedule_and_publish() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let provider = Address::generate(&env);
    let signal_data = make_v2_signal(&env);

    let current_time = env.ledger().timestamp();
    let publish_at = current_time + 60;

    let recurrence = RecurrencePattern {
        is_recurring: false,
        interval_seconds: 0,
        repeat_count: 0,
    };

    let schedule_id = client.schedule(&provider, &signal_data, &publish_at, &recurrence);
    assert_eq!(schedule_id, 0);

    env.ledger().set_timestamp(publish_at + 1);

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

    let signal_data = SignalDataV2 {
        asset_pair: String::from_str(&env, "ETH/USD"),
        action: SignalAction::Sell,
        price: 3000_0000000,
        rationale: String::from_str(&env, "Bearish divergence"),
        confidence: 60,
        risk_level: RiskLevel::High,
    };

    let current_time = env.ledger().timestamp();
    let publish_at = current_time + 3600;

    let recurrence = RecurrencePattern {
        is_recurring: false,
        interval_seconds: 0,
        repeat_count: 0,
    };

    let schedule_id = client.schedule(&provider, &signal_data, &publish_at, &recurrence);

    client.cancel_schedule(&provider, &schedule_id);

    env.ledger().set_timestamp(publish_at + 1);
    let published_ids = client.trigger_scheduled_publications();

    assert_eq!(published_ids.len(), 0);
}

/// Verifies that a record seeded with the legacy V1 shape is transparently
/// upgraded to V2 on read via `VersionedSignalData::resolve()` (Issue #568).
/// This test simulates "existing stored data under the old shape" by directly
/// writing a `VersionedSignalData::V1` record into persistent storage,
/// bypassing the `schedule()` entrypoint which always writes V2.
#[test]
fn test_v1_signal_data_upgrades_on_read() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);

    env.as_contract(&contract_id, || {
        let provider = Address::generate(&env);

        // Construct a V1 record directly — the old 4-field shape.
        let v1_data = SignalDataV1 {
            asset_pair: String::from_str(&env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100_000i128,
            rationale: String::from_str(&env, "legacy signal"),
        };
        let v1_scheduled = ScheduledSignal {
            id: 0,
            provider: provider.clone(),
            signal_data: VersionedSignalData::V1(v1_data.clone()),
            publish_at: 9999,
            recurrence: RecurrencePattern {
                is_recurring: false,
                interval_seconds: 0,
                repeat_count: 0,
            },
            status: ScheduleStatus::Pending,
        };

        // Seed into persistent storage, bypassing the normal schedule path.
        env.storage()
            .persistent()
            .set(&ScheduleDataKey::Schedule(0u64), &v1_scheduled);
        env.storage()
            .instance()
            .set(&ScheduleDataKey::NextScheduleId, &1u64);

        // Read back and resolve — should yield V2 with default confidence/risk.
        let (record, resolved) = get_scheduled_signal_data(&env, 0).expect("record must exist");

        // Verify the raw stored variant is still V1.
        assert!(
            matches!(record.signal_data, VersionedSignalData::V1(_)),
            "raw stored data must remain V1 (no write-back on read)"
        );

        // Verify V2 resolve applies correct defaults.
        assert_eq!(resolved.asset_pair, v1_data.asset_pair);
        assert_eq!(resolved.action, v1_data.action);
        assert_eq!(resolved.price, v1_data.price);
        assert_eq!(resolved.rationale, v1_data.rationale);
        assert_eq!(resolved.confidence, 50, "default confidence for V1 upgrade");
        assert_eq!(
            resolved.risk_level,
            RiskLevel::Medium,
            "default risk_level for V1 upgrade"
        );
    });
}

/// Verifies that a V2 record round-trips through storage intact and that
/// the versioned enum coexists with V1 records without conflict (Issue #568).
#[test]
fn test_v2_signal_data_roundtrips() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);

    env.as_contract(&contract_id, || {
        let provider = Address::generate(&env);

        let v2_data = SignalDataV2 {
            asset_pair: String::from_str(&env, "BTC/USDC"),
            action: SignalAction::Sell,
            price: 65_000_0000000i128,
            rationale: String::from_str(&env, "breakdown below key support"),
            confidence: 75,
            risk_level: RiskLevel::High,
        };

        let v2_scheduled = ScheduledSignal {
            id: 1,
            provider: provider.clone(),
            signal_data: VersionedSignalData::V2(v2_data.clone()),
            publish_at: 5000,
            recurrence: RecurrencePattern {
                is_recurring: false,
                interval_seconds: 0,
                repeat_count: 0,
            },
            status: ScheduleStatus::Pending,
        };

        env.storage()
            .persistent()
            .set(&ScheduleDataKey::Schedule(1u64), &v2_scheduled);
        env.storage()
            .instance()
            .set(&ScheduleDataKey::NextScheduleId, &2u64);

        let (_record, resolved) = get_scheduled_signal_data(&env, 1).expect("record must exist");

        // V2 resolve is a no-op — all fields preserved exactly.
        assert_eq!(resolved.asset_pair, v2_data.asset_pair);
        assert_eq!(resolved.confidence, 75);
        assert_eq!(resolved.risk_level, RiskLevel::High);
    });
}
