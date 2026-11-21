#![cfg(test)]
use super::*;
use crate::categories::{RiskLevel, SignalCategory};
use crate::types::{Signal, SignalAction, SignalStatus};
use crate::versioning::{CopyRecord, SignalVersion};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

fn create_test_signal(env: &Env, provider: Address, signal_id: u64) -> Signal {
    Signal {
        id: signal_id,
        provider,
        asset_pair: String::from_str(env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100,
        rationale: String::from_str(env, "Initial rationale"),
        timestamp: env.ledger().timestamp(),
        expiry: env.ledger().timestamp() + 86400,
        status: SignalStatus::Active,
        executions: 0,
        successful_executions: 0,
        total_volume: 0,
        total_roi: 0,
        category: SignalCategory::SWING,
        tags: soroban_sdk::Vec::new(env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
        submitted_at: env.ledger().timestamp(),
        rationale_hash: String::from_str(env, "Initial rationale"),
        confidence: 50,
        adoption_count: 0,
        ai_validation_score: None,
        avg_copier_roi_bps: 0,
        copier_closed_count: 0,
    }
}

#[test]
fn test_update_signal_price() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let new_version = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    assert_eq!(new_version, 2);
    assert_eq!(signal.price, 150);

    // Verify version history
    let history = versioning::get_signal_history(&env, signal_id);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().price, 100);
    assert_eq!(history.get(0).unwrap().version, 1);
    });
}

#[test]
fn test_update_signal_rationale() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let new_rationale = String::from_str(&env, "Updated rationale");
    let new_version = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        None,
        Some(new_rationale.clone()),
        None,
        &mut signal,
    )
    .unwrap();

    assert_eq!(new_version, 2);
    assert_eq!(signal.rationale, new_rationale);
    });
}

#[test]
fn test_update_signal_expiry() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let new_expiry = env.ledger().timestamp() + 172800; // 2 days
    let new_version = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        None,
        None,
        Some(new_expiry),
        &mut signal,
    )
    .unwrap();

    assert_eq!(new_version, 2);
    assert_eq!(signal.expiry, new_expiry);
    });
}

#[test]
fn test_multiple_updates() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    // First update
    env.ledger().with_mut(|li| li.timestamp += 3700); // Past cooldown
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    // Second update
    env.ledger().with_mut(|li| li.timestamp += 3700);
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(200),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    // Third update
    env.ledger().with_mut(|li| li.timestamp += 3700);
    let version = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(250),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    assert_eq!(version, 4);
    assert_eq!(signal.price, 250);

    let history = versioning::get_signal_history(&env, signal_id);
    assert_eq!(history.len(), 3);
    });
}

#[test]
fn test_update_cooldown() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    // First update
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    // Try immediate second update (should fail)
    let result = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(200),
        None,
        None,
        &mut signal,
    );
    assert_eq!(result, Err(crate::errors::VersioningError::UpdateCooldown));
    });
}

#[test]
fn test_max_updates_limit() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    // Perform 5 updates (max allowed)
    for i in 0..5 {
        env.ledger().with_mut(|li| li.timestamp += 3700);
        versioning::update_signal(
            &env,
            signal_id,
            &provider,
            Some(100 + (i * 10) as i128),
            None,
            None,
            &mut signal,
        )
        .unwrap();
    }

    // Try 6th update (should fail)
    env.ledger().with_mut(|li| li.timestamp += 3700);
    let result = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(200),
        None,
        None,
        &mut signal,
    );
    assert_eq!(
        result,
        Err(crate::errors::VersioningError::MaxUpdatesReached)
    );
    });
}

#[test]
fn test_update_not_owner() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let other_user = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let result = versioning::update_signal(
        &env,
        signal_id,
        &other_user,
        Some(150),
        None,
        None,
        &mut signal,
    );
    assert_eq!(result, Err(crate::errors::VersioningError::NotSignalOwner));
    });
}

#[test]
fn test_update_inactive_signal() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);
    signal.status = SignalStatus::Expired;

    let result = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    );
    assert_eq!(
        result,
        Err(crate::errors::VersioningError::CannotUpdateInactive)
    );
    });
}

#[test]
fn test_update_expired_signal() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    // Fast forward past expiry
    env.ledger().with_mut(|li| li.timestamp = signal.expiry + 1);

    let result = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    );
    assert_eq!(result, Err(crate::errors::VersioningError::SignalExpired));
    });
}

#[test]
fn test_invalid_price_update() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let result =
        versioning::update_signal(&env, signal_id, &provider, Some(0), None, None, &mut signal);
    assert_eq!(result, Err(crate::errors::VersioningError::InvalidPrice));

    let result = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(-100),
        None,
        None,
        &mut signal,
    );
    assert_eq!(result, Err(crate::errors::VersioningError::InvalidPrice));
    });
}

#[test]
fn test_invalid_expiry_update() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let past_time = env.ledger().timestamp() - 1000;
    let result = versioning::update_signal(
        &env,
        signal_id,
        &provider,
        None,
        None,
        Some(past_time),
        &mut signal,
    );
    assert_eq!(result, Err(crate::errors::VersioningError::InvalidExpiry));
    });
}

#[test]
fn test_record_copy() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let user = Address::generate(&env);
    let signal_id = 1;

    versioning::record_copy(&env, &user, signal_id, 1);

    let record = versioning::get_copy_record(&env, &user, signal_id).unwrap();
    assert_eq!(record.signal_id, signal_id);
    assert_eq!(record.version, 1);
    assert_eq!(record.user, user);
    });
}

#[test]
fn test_pending_updates() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let user = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    // User copies at version 1
    versioning::record_copy(&env, &user, signal_id, 1);

    // Provider makes 2 updates
    env.ledger().with_mut(|li| li.timestamp += 3700);
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    env.ledger().with_mut(|li| li.timestamp += 3700);
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(200),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    // Check pending updates
    let pending = versioning::get_pending_updates(&env, &user, signal_id);
    assert_eq!(pending.len(), 2);
    assert_eq!(pending.get(0).unwrap(), 2);
    assert_eq!(pending.get(1).unwrap(), 3);
    });
}

#[test]
fn test_mark_notified() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let user = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    versioning::record_copy(&env, &user, signal_id, 1);

    env.ledger().with_mut(|li| li.timestamp += 3700);
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    // Mark as notified
    versioning::mark_notified(&env, &user, signal_id, 2);

    // Check pending updates (should be empty now)
    let pending = versioning::get_pending_updates(&env, &user, signal_id);
    assert_eq!(pending.len(), 0);
    });
}

#[test]
fn test_version_history_order() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    // Make 3 updates with different prices
    let prices = [150, 200, 250];
    for (i, &price) in prices.iter().enumerate() {
        env.ledger().with_mut(|li| li.timestamp += 3700);
        versioning::update_signal(
            &env,
            signal_id,
            &provider,
            Some(price),
            None,
            None,
            &mut signal,
        )
        .unwrap();
    }

    let history = versioning::get_signal_history(&env, signal_id);
    assert_eq!(history.len(), 3);

    // Verify versions are in order
    for (i, version_record) in history.iter().enumerate() {
        assert_eq!(version_record.version, (i + 1) as u32);
    }
    });
}

#[test]
fn test_get_latest_version() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    // Initial version
    assert_eq!(versioning::get_latest_version(&env, signal_id), 1);

    // After update
    env.ledger().with_mut(|li| li.timestamp += 3700);
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    )
    .unwrap();
    assert_eq!(versioning::get_latest_version(&env, signal_id), 2);
    });
}

#[test]
fn test_get_update_count() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    assert_eq!(versioning::get_update_count(&env, signal_id), 0);

    env.ledger().with_mut(|li| li.timestamp += 3700);
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(150),
        None,
        None,
        &mut signal,
    )
    .unwrap();
    assert_eq!(versioning::get_update_count(&env, signal_id), 1);

    env.ledger().with_mut(|li| li.timestamp += 3700);
    versioning::update_signal(
        &env,
        signal_id,
        &provider,
        Some(200),
        None,
        None,
        &mut signal,
    )
    .unwrap();
    assert_eq!(versioning::get_update_count(&env, signal_id), 2);
    });
}
    });
}

// ── edit_signal_audit tests (Issue #686) ──────────────────────────────────

#[test]
fn test_edit_signal_audit_stores_prior_version() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);
    let original_price = signal.price;

    let new_version = versioning::edit_signal_with_audit(
        &env,
        signal_id,
        &provider,
        Some(200),
        None,
        None,
        &mut signal,
    )
    .unwrap();

    assert_eq!(new_version, 2);
    assert_eq!(signal.price, 200);

    let history = versioning::get_signal_history(&env, signal_id);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().price, original_price);
    assert_eq!(history.get(0).unwrap().version, 1);
    });
}

#[test]
fn test_edit_signal_audit_multiple_edits_create_versions() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

#[test]
fn test_edit_signal_audit_blocked_after_copy_trade() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);
    signal.adoption_count = 1;

    let result = versioning::edit_signal_with_audit(
        &env, signal_id, &provider, Some(200), None, None, &mut signal,
    );
    assert_eq!(result, Err(crate::errors::SignalEditError::SignalAlreadyCopied));
    });
}

#[test]
fn test_edit_signal_audit_blocked_for_non_owner() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let other = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let result = versioning::edit_signal_with_audit(
        &env, signal_id, &other, Some(200), None, None, &mut signal,
    );
    assert_eq!(result, Err(crate::errors::SignalEditError::NotSignalOwner));
    });
}

#[test]
fn test_edit_signal_audit_blocked_after_expiry() {
    let env = Env::default();
    env.ledger().set_timestamp(200_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let result = versioning::edit_signal_with_audit(
        &env, signal_id, &provider, Some(200), None, None, &mut signal,
    );
    assert_eq!(result, Err(crate::errors::SignalEditError::EditWindowClosed));
    });
}

#[test]
fn test_edit_signal_audit_rejects_invalid_price() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    let result = versioning::edit_signal_with_audit(
        &env, signal_id, &provider, Some(0), None, None, &mut signal,
    );
    assert_eq!(result, Err(crate::errors::SignalEditError::FieldNotEditable));
    });
}

#[test]
fn test_get_signal_version_history_via_entrypoint() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &registry_cid);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let mut signal = create_test_signal(&env, provider.clone(), 1);

    versioning::edit_signal_with_audit(
        &env, 1, &provider, Some(250), None, None, &mut signal,
    ).unwrap();
    });

    let history = client.get_signal_version_history(&1u64);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().price, 100);
    assert_eq!(history.get(0).unwrap().version, 1);
}

#[test]
fn test_edit_signal_audit_no_copy_trade_allows_edit() {
    let env = Env::default();
    env.ledger().set_timestamp(100_000);
    env.mock_all_auths();
    #[allow(deprecated)]
    let registry_cid = env.register_contract(None, SignalRegistry);
    env.as_contract(&registry_cid, || {

    let provider = Address::generate(&env);
    let signal_id = 1;
    let mut signal = create_test_signal(&env, provider.clone(), signal_id);

    assert_eq!(signal.adoption_count, 0);

    let result = versioning::edit_signal_with_audit(
        &env, signal_id, &provider, Some(150), None, None, &mut signal,
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 2);
    });
}
    // Edit 1: change price
    let v2 = versioning::edit_signal_with_audit(
        &env, signal_id, &provider, Some(200), None, None, &mut signal,
    ).unwrap();
    assert_eq!(v2, 2);

    // Edit 2: change rationale
    let new_rationale = soroban_sdk::String::from_str(&env, "Updated rationale");
    let v3 = versioning::edit_signal_with_audit(
        &env, signal_id, &provider, None, Some(new_rationale.clone()), None, &mut signal,
    ).unwrap();
    assert_eq!(v3, 3);

    // Edit 3: change expiry
    env.ledger().with_mut(|li| li.timestamp += 1000);
    let v4 = versioning::edit_signal_with_audit(
        &env, signal_id, &provider, None, None, Some(200_000), &mut signal,
    ).unwrap();
    assert_eq!(v4, 4);

    let history = versioning::get_signal_history(&env, signal_id);
    assert_eq!(history.len(), 3);
    assert_eq!(history.get(0).unwrap().version, 1);
    assert_eq!(history.get(0).unwrap().price, 100);
    assert_eq!(history.get(1).unwrap().version, 2);
    assert_eq!(history.get(1).unwrap().price, 200);
    assert_eq!(history.get(2).unwrap().version, 3);
    });
}
