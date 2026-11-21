#![cfg(test)]
//! Issue #1432: independent partial unbonding requests.
//! Issue #1433: slashing decisions with evidence references.

use crate::{
    migration::{MigrationKey, StakeInfoV2},
    slash_evidence::{MAX_DECISION_REASON_LEN, MAX_EVIDENCE_LEN},
    unbonding::MAX_PENDING_UNBONDING_REQUESTS,
    SlashSeverity, StakeVaultContract, StakeVaultContractClient, StakeVaultError,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::{self, StellarAssetClient},
    Address, Bytes, Env, Map, String, Symbol,
};

const WITHDRAWAL_COOLDOWN: u64 = 3_600;

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
    (env, vault_id, token, signal_registry)
}

fn seed_stake(env: &Env, vault: &Address, token: &Address, staker: &Address, balance: i128, locked_until: u64) {
    StellarAssetClient::new(env, token).mint(vault, &balance);
    env.as_contract(vault, || {
        let mut stakes: Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| Map::new(env));
        stakes.set(
            staker.clone(),
            StakeInfoV2 {
                balance,
                locked_until,
                last_updated: 42,
            },
        );
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);
    });
}

fn stake_info(env: &Env, vault: &Address, staker: &Address) -> StakeInfoV2 {
    env.as_contract(vault, || {
        let stakes: Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap();
        stakes.get(staker.clone()).unwrap()
    })
}

// ── Issue #1432 ───────────────────────────────────────────────────────────────

#[test]
fn concurrent_partial_requests_track_own_amount_and_unlock_time() {
    let (env, vault, token, _registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let staker = Address::generate(&env);
    let locked_until = 500;
    seed_stake(&env, &vault, &token, &staker, 10_000, locked_until);

    env.ledger().set_timestamp(100);
    let r0 = client.request_unbonding(&staker, &1_000);
    env.ledger().set_timestamp(2_000);
    let r1 = client.request_unbonding(&staker, &2_500);
    assert_ne!(r0, r1);

    let req0 = client.get_unbonding_request(&staker, &r0).unwrap();
    let req1 = client.get_unbonding_request(&staker, &r1).unwrap();
    assert_eq!(req0.amount, 1_000);
    assert_eq!(req0.unlock_at, 100 + WITHDRAWAL_COOLDOWN);
    assert_eq!(req1.amount, 2_500);
    assert_eq!(req1.unlock_at, 2_000 + WITHDRAWAL_COOLDOWN);
    assert_eq!(client.get_pending_unbonding(&staker).len(), 2);
    assert_eq!(client.get_unbonding_total(&staker), 3_500);

    // The remainder keeps its original accounting basis.
    let info = stake_info(&env, &vault, &staker);
    assert_eq!(info.balance, 6_500);
    assert_eq!(info.locked_until, locked_until);
    assert_eq!(info.last_updated, 42);
    assert_eq!(client.get_stake(&staker), 6_500);

    // r0 unlocks first and is withdrawn independently; r1 is still locked.
    env.ledger().set_timestamp(100 + WITHDRAWAL_COOLDOWN);
    assert_eq!(client.withdraw_unbonded(&staker, &r0), 1_000);
    assert_eq!(
        client.try_withdraw_unbonded(&staker, &r1),
        Err(Ok(StakeVaultError::TimelockNotElapsed))
    );
    // A request is paid at most once.
    assert_eq!(
        client.try_withdraw_unbonded(&staker, &r0),
        Err(Ok(StakeVaultError::NoUnstakeQueued))
    );
    assert_eq!(token::Client::new(&env, &token).balance(&staker), 1_000);
    assert_eq!(client.get_stake(&staker), 6_500);
}

#[test]
fn unlock_time_respects_position_lock() {
    let (env, vault, token, _registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let staker = Address::generate(&env);
    let locked_until = 1_000_000;
    seed_stake(&env, &vault, &token, &staker, 10_000, locked_until);

    let id = client.request_unbonding(&staker, &1_000);
    assert_eq!(
        client.get_unbonding_request(&staker, &id).unwrap().unlock_at,
        locked_until
    );
}

#[test]
fn final_withdrawal_drains_all_requests() {
    let (env, vault, token, _registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let staker = Address::generate(&env);
    seed_stake(&env, &vault, &token, &staker, 9_000, 0);

    client.request_unbonding(&staker, &3_000);
    client.request_unbonding(&staker, &3_000);
    client.request_unbonding(&staker, &3_000);
    assert_eq!(client.get_stake(&staker), 0);
    // Nothing more to unbond.
    assert_eq!(
        client.try_request_unbonding(&staker, &1),
        Err(Ok(StakeVaultError::NoStake))
    );
    assert_eq!(
        client.try_withdraw_all_unbonded(&staker),
        Err(Ok(StakeVaultError::NoUnstakeQueued))
    );

    env.ledger().set_timestamp(WITHDRAWAL_COOLDOWN);
    assert_eq!(client.withdraw_all_unbonded(&staker), 9_000);
    assert_eq!(token::Client::new(&env, &token).balance(&staker), 9_000);
    assert_eq!(client.get_pending_unbonding(&staker).len(), 0);
    assert_eq!(client.get_unbonding_total(&staker), 0);
    assert_eq!(
        client.try_withdraw_all_unbonded(&staker),
        Err(Ok(StakeVaultError::NoUnstakeQueued))
    );
}

#[test]
fn unbonding_requests_are_validated_and_bounded() {
    let (env, vault, token, _registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let staker = Address::generate(&env);
    seed_stake(&env, &vault, &token, &staker, 1_000, 0);

    assert_eq!(
        client.try_request_unbonding(&staker, &0),
        Err(Ok(StakeVaultError::InvalidAmount))
    );
    assert_eq!(
        client.try_request_unbonding(&staker, &1_001),
        Err(Ok(StakeVaultError::InvalidAmount))
    );
    for _ in 0..MAX_PENDING_UNBONDING_REQUESTS {
        client.request_unbonding(&staker, &10);
    }
    assert_eq!(
        client.try_request_unbonding(&staker, &10),
        Err(Ok(StakeVaultError::QueueFull))
    );
    assert_eq!(
        client.get_stake(&staker),
        1_000 - 10 * MAX_PENDING_UNBONDING_REQUESTS as i128
    );
}

// ── Issue #1433 ───────────────────────────────────────────────────────────────

fn evidence(env: &Env, len: u32, fill: u8) -> Bytes {
    let mut b = Bytes::new(env);
    for _ in 0..len {
        b.push_back(fill);
    }
    b
}

fn long_reason(env: &Env, len: u32) -> String {
    let mut buf = [b'r'; (MAX_DECISION_REASON_LEN + 1) as usize];
    let len = core::cmp::min(len as usize, buf.len());
    buf[0] = b'x';
    String::from_bytes(env, &buf[..len])
}

#[test]
fn slash_with_evidence_writes_immutable_audit_record() {
    let (env, vault, token, registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let provider = Address::generate(&env);
    seed_stake(&env, &vault, &token, &provider, 1_000_000, 0);

    let ev = evidence(&env, 32, 7);
    let reason = String::from_str(&env, "double-signed signal 42");
    let amount = client.slash_stake_with_evidence(
        &registry,
        &provider,
        &SlashSeverity::Major,
        &Symbol::new(&env, "fraud"),
        &ev,
        &reason,
    );
    assert_eq!(amount, 300_000);
    assert_eq!(client.get_stake(&provider), 700_000);

    let record = client.get_slash_record(&0).unwrap();
    let audit = client.get_slash_evidence(&0).unwrap();
    assert_eq!(audit.slash_id, record.slash_id);
    assert_eq!(audit.provider, provider);
    assert_eq!(audit.evidence, ev);
    assert_eq!(audit.decision_reason, reason);
    assert_eq!(audit.slash_amount, record.slash_amount);
}

#[test]
fn invalid_evidence_size_is_rejected_without_slashing() {
    let (env, vault, token, registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let provider = Address::generate(&env);
    seed_stake(&env, &vault, &token, &provider, 1_000_000, 0);
    let reason = String::from_str(&env, "reason");
    let sev = SlashSeverity::Minor;
    let sym = Symbol::new(&env, "fraud");

    for bad in [evidence(&env, 0, 1), evidence(&env, MAX_EVIDENCE_LEN + 1, 1)] {
        assert_eq!(
            client.try_slash_stake_with_evidence(&registry, &provider, &sev, &sym, &bad, &reason),
            Err(Ok(StakeVaultError::InvalidEvidence))
        );
    }
    let ok_ev = evidence(&env, MAX_EVIDENCE_LEN, 1);
    for bad_reason in [
        String::from_str(&env, ""),
        long_reason(&env, MAX_DECISION_REASON_LEN + 1),
    ] {
        assert_eq!(
            client.try_slash_stake_with_evidence(&registry, &provider, &sev, &sym, &ok_ev, &bad_reason),
            Err(Ok(StakeVaultError::InvalidEvidence))
        );
    }

    // Nothing was slashed or recorded.
    assert_eq!(client.get_stake(&provider), 1_000_000);
    assert!(client.get_slash_record(&0).is_none());
    assert!(client.get_slash_evidence(&0).is_none());

    // The maximum sizes themselves are accepted.
    client.slash_stake_with_evidence(
        &registry,
        &provider,
        &sev,
        &sym,
        &ok_ev,
        &long_reason(&env, MAX_DECISION_REASON_LEN),
    );
    assert!(client.get_slash_evidence(&0).is_some());
}

#[test]
fn duplicate_decision_is_rejected_atomically() {
    let (env, vault, token, registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let first = Address::generate(&env);
    let second = Address::generate(&env);
    seed_stake(&env, &vault, &token, &first, 1_000_000, 0);
    seed_stake(&env, &vault, &token, &second, 1_000_000, 0);

    let ev = evidence(&env, 32, 9);
    let reason = String::from_str(&env, "decision");
    let sym = Symbol::new(&env, "fraud");
    client.slash_stake_with_evidence(&registry, &first, &SlashSeverity::Minor, &sym, &ev, &reason);
    let audit_before = client.get_slash_evidence(&0).unwrap();

    // Same evidence again — for the same or another provider — is a duplicate
    // decision and changes nothing.
    for provider in [&first, &second] {
        assert_eq!(
            client.try_slash_stake_with_evidence(
                &registry,
                provider,
                &SlashSeverity::Critical,
                &sym,
                &ev,
                &String::from_str(&env, "rewritten reason"),
            ),
            Err(Ok(StakeVaultError::DuplicateSlashDecision))
        );
    }
    assert_eq!(client.get_stake(&second), 1_000_000);
    assert!(client.get_slash_record(&1).is_none());
    assert_eq!(client.get_slash_evidence(&0).unwrap(), audit_before);
}

#[test]
fn slash_with_evidence_requires_signal_registry() {
    let (env, vault, token, _registry) = setup();
    let client = StakeVaultContractClient::new(&env, &vault);
    let provider = Address::generate(&env);
    seed_stake(&env, &vault, &token, &provider, 1_000_000, 0);
    assert_eq!(
        client.try_slash_stake_with_evidence(
            &Address::generate(&env),
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "fraud"),
            &evidence(&env, 8, 1),
            &String::from_str(&env, "reason"),
        ),
        Err(Ok(StakeVaultError::Unauthorized))
    );
    assert!(client.get_slash_evidence(&0).is_none());
}
