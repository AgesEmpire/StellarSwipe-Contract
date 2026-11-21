//! Tests for issues #1395 (single active unstake reservation), #1396
//! (idempotent queue completion), #1397 (TTL renewal before critical state
//! transitions) and #1398 (authorization config preserved across upgrades).

#![cfg(test)]

use crate::{
    auth_config::{self, AuthConfigSnapshot},
    emergency_unstake::EmergencyMultiSigConfig,
    migration::{MigrationKey, StakeInfoV2},
    ttl_guard, StakeVaultContract, StakeVaultContractClient, StakeVaultError, StorageKey,
    UnstakeCompletion, UnstakeRequest,
};
use shared::multisig::{MultisigConfig, MultisigStorageKey};
use soroban_sdk::{
    testutils::{
        storage::{Instance as _, Persistent as _},
        Address as _, Ledger as _,
    },
    token::{StellarAssetClient, TokenClient},
    vec, Address, BytesN, Env, Map,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signal_registry = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let vault_id = env.register(StakeVaultContract, ());
    StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token, &signal_registry);
    (env, vault_id, token, admin)
}

fn seed(env: &Env, vault_id: &Address, staker: &Address, balance: i128) {
    env.as_contract(vault_id, || {
        let mut stakes: Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| Map::new(env));
        stakes.set(
            staker.clone(),
            StakeInfoV2 {
                balance,
                locked_until: 0,
                last_updated: env.ledger().timestamp(),
            },
        );
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);
    });
}

fn advance_ledgers(env: &Env, ledgers: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = li.sequence_number.saturating_add(ledgers);
    });
}

// ── Issue #1395: one active reservation per stake position ────────────────────

#[test]
fn retry_while_active_is_rejected_and_reserves_once() {
    let (env, vault_id, _token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    let other = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    seed(&env, &vault_id, &other, 1_000_000);

    assert_eq!(client.queue_unstake(&staker), 0);
    assert_eq!(client.get_unstake_reservation(&staker), 1_000_000);

    // Retries are rejected and do not consume a queue slot.
    assert_eq!(
        client.try_queue_unstake(&staker),
        Err(Ok(StakeVaultError::UnstakeAlreadyQueued))
    );
    assert_eq!(
        client.try_queue_unstake(&staker),
        Err(Ok(StakeVaultError::UnstakeAlreadyQueued))
    );
    assert_eq!(client.get_unstake_reservation(&staker), 1_000_000);
    assert_eq!(client.get_queue_position(&staker), Some(0));
    assert_eq!(client.queue_unstake(&other), 1);
}

#[test]
fn reserved_stake_cannot_be_withdrawn_through_another_path() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &1_000_000);

    client.queue_unstake(&staker);

    assert_eq!(
        client.try_withdraw_stake(&staker),
        Err(Ok(StakeVaultError::UnstakeAlreadyQueued))
    );
    assert_eq!(
        client.try_partial_unstake(&staker, &1_000),
        Err(Ok(StakeVaultError::UnstakeAlreadyQueued))
    );
    assert_eq!(client.get_stake(&staker), 1_000_000);
    assert_eq!(client.get_unstake_reservation(&staker), 1_000_000);
}

#[test]
fn reservation_never_exceeds_withdrawable_stake() {
    let (env, vault_id, _token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);

    client.queue_unstake(&staker);
    // Position shrinks after the request (e.g. a slash).
    seed(&env, &vault_id, &staker, 400_000);

    assert_eq!(client.get_unstake_reservation(&staker), 400_000);
}

#[test]
fn partial_queue_completion_releases_only_processed_reservations() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    seed(&env, &vault_id, &a, 1_000_000);
    seed(&env, &vault_id, &b, 2_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &3_000_000);

    client.queue_unstake(&a);
    client.queue_unstake(&b);

    assert_eq!(client.process_unstake_queue(&1), 1);

    assert_eq!(client.get_unstake_reservation(&a), 0);
    assert_eq!(client.get_queue_position(&a), None);
    assert_eq!(client.get_unstake_reservation(&b), 2_000_000);
    assert_eq!(client.get_queue_position(&b), Some(0));
    assert_eq!(
        client.try_queue_unstake(&b),
        Err(Ok(StakeVaultError::UnstakeAlreadyQueued))
    );
}

#[test]
fn new_request_allowed_after_previous_request_closed() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &1_500_000);

    assert_eq!(client.queue_unstake(&staker), 0);
    assert_eq!(client.process_unstake_queue(&1), 1);

    // Position re-funded after closure — a fresh request is accepted.
    seed(&env, &vault_id, &staker, 500_000);
    assert_eq!(client.queue_unstake(&staker), 1);
    assert_eq!(client.get_unstake_reservation(&staker), 500_000);
}

#[test]
fn stale_pointer_does_not_block_new_request() {
    let (env, vault_id, _token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);

    // Pointer left behind for a ticket whose entry no longer exists.
    env.as_contract(&vault_id, || {
        env.storage()
            .persistent()
            .set(&StorageKey::UserUnstakeTicket(staker.clone()), &42u64);
    });
    assert_eq!(client.get_unstake_reservation(&staker), 0);

    assert_eq!(client.queue_unstake(&staker), 0);
    assert_eq!(client.get_queue_position(&staker), Some(0));
}

// ── Issue #1396: idempotent queue completion ──────────────────────────────────

#[test]
fn completion_is_recorded_and_retry_does_not_pay_again() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &2_000_000);

    client.queue_unstake(&staker);
    assert_eq!(client.process_unstake_queue(&5), 1);

    let completion: UnstakeCompletion = client.get_unstake_completion(&0).unwrap();
    assert_eq!(completion.ticket, 0);
    assert_eq!(completion.user, staker);
    assert_eq!(completion.amount, 1_000_000);

    // Retrying the processing call is a no-op.
    assert_eq!(
        client.try_process_unstake_queue(&5),
        Err(Ok(StakeVaultError::QueueEmpty))
    );
    assert_eq!(TokenClient::new(&env, &token).balance(&staker), 1_000_000);
}

#[test]
fn replayed_completed_ticket_is_never_paid_twice() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &3_000_000);

    client.queue_unstake(&staker);
    assert_eq!(client.process_unstake_queue(&1), 1);

    // Conflicting state: the completed ticket's entry, pointer and head are
    // restored and the position is funded again.
    seed(&env, &vault_id, &staker, 1_000_000);
    env.as_contract(&vault_id, || {
        env.storage().persistent().set(
            &StorageKey::UnstakeQueueEntry(0),
            &UnstakeRequest {
                ticket: 0,
                user: staker.clone(),
                queued_at: 0,
            },
        );
        env.storage()
            .persistent()
            .set(&StorageKey::UserUnstakeTicket(staker.clone()), &0u64);
        env.storage()
            .instance()
            .set(&StorageKey::UnstakeQueueHead, &0u64);
    });

    assert_eq!(client.process_unstake_queue(&1), 0);
    assert_eq!(TokenClient::new(&env, &token).balance(&staker), 1_000_000);
    assert_eq!(client.get_stake(&staker), 1_000_000);
    assert_eq!(client.get_unstake_completion(&0).unwrap().amount, 1_000_000);
    assert_eq!(client.get_queue_position(&staker), None);
}

#[test]
fn entry_not_matching_user_pointer_is_dropped_without_payment() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &2_000_000);

    // Ticket 0 is the staker's real request.
    client.queue_unstake(&staker);
    // A duplicate entry for the same staker at ticket 1 (conflicting state).
    env.as_contract(&vault_id, || {
        env.storage().persistent().set(
            &StorageKey::UnstakeQueueEntry(1),
            &UnstakeRequest {
                ticket: 1,
                user: staker.clone(),
                queued_at: 0,
            },
        );
        env.storage()
            .instance()
            .set(&StorageKey::UnstakeQueueTail, &2u64);
    });

    assert_eq!(client.process_unstake_queue(&5), 1);
    assert_eq!(TokenClient::new(&env, &token).balance(&staker), 1_000_000);
    assert!(client.get_unstake_completion(&1).is_none());
}

#[test]
fn emptied_position_closes_ticket_without_blocking_queue() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    seed(&env, &vault_id, &a, 1_000_000);
    seed(&env, &vault_id, &b, 1_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &1_000_000);

    client.queue_unstake(&a);
    client.queue_unstake(&b);
    // A's position is emptied by another path (e.g. slashed to zero).
    seed(&env, &vault_id, &a, 0);

    assert_eq!(client.process_unstake_queue(&5), 2);
    assert_eq!(client.get_unstake_completion(&0).unwrap().amount, 0);
    assert_eq!(client.get_unstake_completion(&1).unwrap().amount, 1_000_000);
    assert_eq!(TokenClient::new(&env, &token).balance(&a), 0);
    assert_eq!(TokenClient::new(&env, &token).balance(&b), 1_000_000);
}

// ── Issue #1397: TTL renewal before critical state transitions ────────────────

#[test]
fn near_threshold_entries_are_renewed_by_queue_unstake() {
    let (env, vault_id, _token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);

    // Drive the stake map to the edge of expiry.
    let before = env.as_contract(&vault_id, || {
        env.storage().persistent().get_ttl(&MigrationKey::StakesV2)
    });
    assert!(before < ttl_guard::CRITICAL_TTL_THRESHOLD_LEDGERS);
    advance_ledgers(&env, before.saturating_sub(10));

    client.queue_unstake(&staker);

    env.as_contract(&vault_id, || {
        let p = env.storage().persistent();
        assert_eq!(
            p.get_ttl(&MigrationKey::StakesV2),
            ttl_guard::CRITICAL_TTL_TARGET_LEDGERS
        );
        assert_eq!(
            p.get_ttl(&StorageKey::UnstakeQueueEntry(0)),
            ttl_guard::CRITICAL_TTL_TARGET_LEDGERS
        );
        assert_eq!(
            p.get_ttl(&StorageKey::UserUnstakeTicket(staker.clone())),
            ttl_guard::CRITICAL_TTL_TARGET_LEDGERS
        );
        assert_eq!(
            p.get_ttl(&StorageKey::UnstakeReservation(staker.clone())),
            ttl_guard::CRITICAL_TTL_TARGET_LEDGERS
        );
        assert_eq!(
            env.storage().instance().get_ttl(),
            ttl_guard::CRITICAL_TTL_TARGET_LEDGERS
        );
    });
}

#[test]
fn stake_map_near_expiry_is_renewed_before_processing() {
    let (env, vault_id, token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &1_000_000);

    // Drive the stake map to the edge of expiry, then queue and process.
    let before = env.as_contract(&vault_id, || {
        env.storage().persistent().get_ttl(&MigrationKey::StakesV2)
    });
    assert!(before < ttl_guard::CRITICAL_TTL_THRESHOLD_LEDGERS);
    advance_ledgers(&env, before.saturating_sub(10));
    client.queue_unstake(&staker);

    assert_eq!(client.process_unstake_queue(&1), 1);
    env.as_contract(&vault_id, || {
        assert_eq!(
            env.storage()
                .persistent()
                .get_ttl(&StorageKey::UnstakeCompletion(0)),
            ttl_guard::CRITICAL_TTL_TARGET_LEDGERS
        );
        assert_eq!(
            env.storage().persistent().get_ttl(&MigrationKey::StakesV2),
            ttl_guard::CRITICAL_TTL_TARGET_LEDGERS
        );
    });
}

#[test]
fn rejected_transition_persists_nothing() {
    let (env, vault_id, _token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    client.queue_unstake(&staker);

    let tail_before: u64 = env.as_contract(&vault_id, || {
        env.storage()
            .instance()
            .get(&StorageKey::UnstakeQueueTail)
            .unwrap()
    });
    assert!(client.try_queue_unstake(&staker).is_err());
    let tail_after: u64 = env.as_contract(&vault_id, || {
        env.storage()
            .instance()
            .get(&StorageKey::UnstakeQueueTail)
            .unwrap()
    });
    assert_eq!(tail_before, tail_after);
}

#[test]
fn renewal_is_clamped_to_network_max_ttl() {
    let (env, vault_id, _token, _admin) = setup();
    let staker = Address::generate(&env);
    seed(&env, &vault_id, &staker, 1_000_000);
    env.ledger().set_max_entry_ttl(50_000);

    env.as_contract(&vault_id, || {
        ttl_guard::renew_persistent(&env, &MigrationKey::StakesV2);
        assert_eq!(
            env.storage().persistent().get_ttl(&MigrationKey::StakesV2),
            env.storage().max_ttl()
        );
    });
}

// ── Issue #1398: authorization config preserved across migration ──────────────

fn configure_current_layout(
    env: &Env,
    client: &StakeVaultContractClient,
    admin: &Address,
) -> (Address, MultisigConfig, EmergencyMultiSigConfig) {
    let governance = Address::generate(env);
    client.set_governance(admin, &governance);

    let approvers = vec![env, Address::generate(env), Address::generate(env)];
    client.configure_emergency_multisig(admin, &approvers, &2, &500, &3_600);
    let emergency = EmergencyMultiSigConfig {
        admins: approvers,
        required: 2,
        penalty_bps: 500,
        timeout_secs: 3_600,
    };

    let multisig = MultisigConfig {
        signers: vec![
            env,
            admin.clone(),
            Address::generate(env),
            Address::generate(env),
        ],
        threshold: 2,
        proposal_timeout_secs: 86_400,
    };
    client.set_multisig_config(admin, &Some(multisig.clone()));
    (governance, multisig, emergency)
}

#[test]
fn legacy_layout_is_captured_and_preserved_exactly() {
    let (env, vault_id, _token, admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);

    let expected = AuthConfigSnapshot {
        admin: admin.clone(),
        governance: None,
        multisig: None,
        emergency: None,
    };
    assert_eq!(client.get_auth_config(), expected);

    env.as_contract(&vault_id, || {
        auth_config::record_pre_upgrade(&env).unwrap();
    });
    assert_eq!(client.verify_auth_config(), expected);
    assert_eq!(client.get_pre_upgrade_auth_config(), Some(expected));
}

#[test]
fn current_layout_is_captured_and_preserved_exactly() {
    let (env, vault_id, _token, admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let (governance, multisig, emergency) = configure_current_layout(&env, &client, &admin);

    let expected = AuthConfigSnapshot {
        admin: admin.clone(),
        governance: Some(governance),
        multisig: Some(multisig),
        emergency: Some(emergency),
    };
    assert_eq!(client.get_auth_config(), expected);

    env.as_contract(&vault_id, || {
        auth_config::record_pre_upgrade(&env).unwrap();
    });
    assert_eq!(client.verify_auth_config(), expected);
}

#[test]
fn drift_after_recorded_upgrade_is_rejected() {
    let (env, vault_id, _token, admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    configure_current_layout(&env, &client, &admin);

    env.as_contract(&vault_id, || {
        auth_config::record_pre_upgrade(&env).unwrap();
        // A migration step silently swaps the guardian.
        env.storage()
            .instance()
            .set(&StorageKey::GovernanceAddress, &Address::generate(&env));
    });

    assert_eq!(
        client.try_verify_auth_config(),
        Err(Ok(StakeVaultError::Unauthorized))
    );
}

#[test]
fn malformed_multisig_config_is_rejected_and_blocks_upgrade() {
    let (env, vault_id, _token, admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);

    env.as_contract(&vault_id, || {
        env.storage().instance().set(
            &MultisigStorageKey::Config,
            &MultisigConfig {
                signers: vec![&env, admin.clone()],
                threshold: 0,
                proposal_timeout_secs: 0,
            },
        );
    });

    assert_eq!(
        client.try_get_auth_config(),
        Err(Ok(StakeVaultError::InvalidMultiSigConfig))
    );
    let next_version = client.get_contract_version() + 1;
    assert_eq!(
        client.try_upgrade(&BytesN::from_array(&env, &[0u8; 32]), &next_version),
        Err(Ok(StakeVaultError::InvalidMultiSigConfig))
    );
    assert_eq!(client.get_pre_upgrade_auth_config(), None);
}

#[test]
fn malformed_emergency_config_is_rejected() {
    let (env, vault_id, _token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);
    let dup = Address::generate(&env);

    env.as_contract(&vault_id, || {
        env.storage().instance().set(
            &StorageKey::EmergencyMultiSigConfig,
            &EmergencyMultiSigConfig {
                admins: vec![&env, dup.clone(), dup.clone()],
                required: 2,
                penalty_bps: 500,
                timeout_secs: 3_600,
            },
        );
    });
    assert_eq!(
        client.try_get_auth_config(),
        Err(Ok(StakeVaultError::InvalidMultiSigConfig))
    );

    env.as_contract(&vault_id, || {
        env.storage().instance().set(
            &StorageKey::EmergencyMultiSigConfig,
            &EmergencyMultiSigConfig {
                admins: vec![&env, dup.clone()],
                required: 1,
                penalty_bps: 10_001,
                timeout_secs: 3_600,
            },
        );
    });
    assert_eq!(
        client.try_get_auth_config(),
        Err(Ok(StakeVaultError::InvalidEmergencyPenalty))
    );
}

#[test]
fn wrong_typed_admin_is_rejected() {
    let (env, vault_id, _token, _admin) = setup();
    let client = StakeVaultContractClient::new(&env, &vault_id);

    env.as_contract(&vault_id, || {
        env.storage().instance().set(&StorageKey::Admin, &7u32);
    });
    assert_eq!(
        client.try_get_auth_config(),
        Err(Ok(StakeVaultError::Unauthorized))
    );
}
