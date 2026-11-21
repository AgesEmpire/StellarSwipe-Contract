#![cfg(test)]
//! Issue #1418: validator reward credits and slash debits emit auditable
//! events, only after the accounting change succeeds, with a stable
//! serialization pinned by fixtures in `event-fixtures/`.
//!
//! # Updating a fixture
//!
//! Only after an intentional schema change (bump
//! `VALIDATOR_EVENT_SCHEMA_VERSION`), run:
//!
//! ```text
//! UPDATE_EVENT_FIXTURES=1 cargo test -p bridge test_validator_accounting
//! ```
//!
//! and commit the regenerated `.hex` files.

extern crate std;

use crate::validator_accounting::{
    self, EvtValidatorRewardCredited, EvtValidatorSlashed, ValidatorAccount,
    ValidatorRewardReason, ValidatorSlashReason, VALIDATOR_EPOCH_SECONDS,
    VALIDATOR_EVENT_SCHEMA_VERSION,
};
use crate::{BridgeContract, BridgeContractClient, BridgeError};
use soroban_sdk::testutils::{Address as _, Events, Ledger as _};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, Env, IntoVal, Map, Symbol, TryFromVal, Val, Vec};

const START: u64 = 10 * VALIDATOR_EPOCH_SECONDS + 5;
const EPOCH: u64 = 10;

struct Setup {
    env: Env,
    client: BridgeContractClient<'static>,
    admin: Address,
    validator: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);
    let id = env.register(BridgeContract, ());
    let client = BridgeContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let mut validators = Vec::new(&env);
    for _ in 0..3 {
        validators.push_back(Address::generate(&env));
    }
    client.initialize(&admin, &validators, &2, &1_000, &1_000, &0);
    let validator = validators.get(0).unwrap();
    Setup {
        env,
        client,
        admin,
        validator,
    }
}

/// Events published under the `(bridge, <name>, ..)` topic prefix.
fn events_named(env: &Env, name: &str) -> std::vec::Vec<(Vec<Val>, Val)> {
    let mut out = std::vec::Vec::new();
    for (_, topics, data) in env.events().all().iter() {
        if topics.len() < 2 {
            continue;
        }
        let t0 = Symbol::try_from_val(env, &topics.get(0).unwrap()).ok();
        let t1 = Symbol::try_from_val(env, &topics.get(1).unwrap()).ok();
        if t0 == Some(Symbol::new(env, "bridge")) && t1 == Some(Symbol::new(env, name)) {
            out.push((topics, data));
        }
    }
    out
}

// ── Accounting + event content ───────────────────────────────────────────────

#[test]
fn reward_credit_updates_balance_and_emits_event() {
    let s = setup();
    let contract_id = s.client.address.clone();
    s.env.as_contract(&contract_id, || {
        let account = validator_accounting::credit_reward(
            &s.env,
            s.validator.clone(),
            250,
            ValidatorRewardReason::RelayFee,
            EPOCH,
        )
        .unwrap();
        assert_eq!(
            account,
            ValidatorAccount {
                balance: 250,
                total_rewarded: 250,
                total_slashed: 0
            }
        );

        let evts = events_named(&s.env, "validator_reward_credited");
        assert_eq!(evts.len(), 1);
        let (topics, data) = &evts[0];
        assert_eq!(
            Address::try_from_val(&s.env, &topics.get(2).unwrap()).unwrap(),
            s.validator
        );
        assert_eq!(u64::try_from_val(&s.env, &topics.get(3).unwrap()).unwrap(), EPOCH);
        let evt = EvtValidatorRewardCredited::try_from_val(&s.env, data).unwrap();
        assert_eq!(
            evt,
            EvtValidatorRewardCredited {
                schema_version: VALIDATOR_EVENT_SCHEMA_VERSION,
                validator: s.validator.clone(),
                amount: 250,
                reason: ValidatorRewardReason::RelayFee,
                epoch: EPOCH,
                balance_after: 250,
                total_rewarded: 250,
            }
        );
    });
}

#[test]
fn slash_debits_balance_and_emits_event() {
    let s = setup();
    let contract_id = s.client.address.clone();
    s.env.as_contract(&contract_id, || {
        validator_accounting::credit_reward(
            &s.env,
            s.validator.clone(),
            500,
            ValidatorRewardReason::Uptime,
            EPOCH - 1,
        )
        .unwrap();
        let account = validator_accounting::slash(
            &s.env,
            s.validator.clone(),
            200,
            ValidatorSlashReason::DoubleSign,
            EPOCH - 1,
        )
        .unwrap();
        assert_eq!(
            account,
            ValidatorAccount {
                balance: 300,
                total_rewarded: 500,
                total_slashed: 200
            }
        );

        let evts = events_named(&s.env, "validator_slashed");
        assert_eq!(evts.len(), 1);
        let evt = EvtValidatorSlashed::try_from_val(&s.env, &evts[0].1).unwrap();
        assert_eq!(evt.validator, s.validator);
        assert_eq!(evt.amount, 200);
        assert_eq!(evt.reason, ValidatorSlashReason::DoubleSign);
        assert_eq!(evt.epoch, EPOCH - 1);
        assert_eq!(evt.balance_after, 300);
        assert_eq!(evt.total_slashed, 200);
    });
}

#[test]
fn events_replay_to_stored_balance() {
    let s = setup();
    let contract_id = s.client.address.clone();
    s.env.as_contract(&contract_id, || {
        validator_accounting::credit_reward(&s.env, s.validator.clone(), 100, ValidatorRewardReason::Attestation, EPOCH).unwrap();
        validator_accounting::credit_reward(&s.env, s.validator.clone(), 40, ValidatorRewardReason::RelayFee, EPOCH).unwrap();
        validator_accounting::slash(&s.env, s.validator.clone(), 70, ValidatorSlashReason::Downtime, EPOCH).unwrap();

        let mut replayed: i128 = 0;
        for (_, data) in events_named(&s.env, "validator_reward_credited") {
            replayed += EvtValidatorRewardCredited::try_from_val(&s.env, &data).unwrap().amount;
        }
        for (_, data) in events_named(&s.env, "validator_slashed") {
            replayed -= EvtValidatorSlashed::try_from_val(&s.env, &data).unwrap().amount;
        }
        assert_eq!(replayed, 70);
        assert_eq!(validator_accounting::get_account(&s.env, &s.validator).balance, replayed);
    });
}

// ── No event on failure ─────────────────────────────────────────────────────

#[test]
fn rejected_changes_emit_no_event_and_leave_balance_unchanged() {
    let s = setup();
    let contract_id = s.client.address.clone();
    s.env.as_contract(&contract_id, || {
        validator_accounting::credit_reward(&s.env, s.validator.clone(), 100, ValidatorRewardReason::RelayFee, EPOCH).unwrap();
        let before = validator_accounting::get_account(&s.env, &s.validator);
        let count_before = s.env.events().all().len();

        assert_eq!(
            validator_accounting::slash(&s.env, s.validator.clone(), 101, ValidatorSlashReason::Downtime, EPOCH),
            Err(BridgeError::InsufficientValidatorBalance)
        );
        assert_eq!(
            validator_accounting::credit_reward(&s.env, s.validator.clone(), 0, ValidatorRewardReason::RelayFee, EPOCH),
            Err(BridgeError::InvalidAmount)
        );
        assert_eq!(
            validator_accounting::slash(&s.env, s.validator.clone(), 10, ValidatorSlashReason::Downtime, EPOCH + 1),
            Err(BridgeError::InvalidValidatorEpoch)
        );

        assert_eq!(s.env.events().all().len(), count_before);
        assert_eq!(validator_accounting::get_account(&s.env, &s.validator), before);
    });
}

#[test]
fn contract_entrypoints_enforce_admin_and_validator_membership() {
    let s = setup();
    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.client.try_credit_validator_reward(&stranger, &s.validator, &10, &ValidatorRewardReason::RelayFee, &EPOCH),
        Err(Ok(BridgeError::UnauthorizedValidator))
    );
    assert_eq!(
        s.client.try_slash_validator(&s.admin, &stranger, &10, &ValidatorSlashReason::DoubleSign, &EPOCH),
        Err(Ok(BridgeError::UnauthorizedValidator))
    );

    let account = s.client.credit_validator_reward(&s.admin, &s.validator, &10, &ValidatorRewardReason::RelayFee, &EPOCH);
    assert_eq!(account.balance, 10);
    assert_eq!(s.client.get_validator_account(&s.validator), account);
    assert_eq!(s.client.get_validator_epoch(), EPOCH);
}

// ── Schema fixtures ─────────────────────────────────────────────────────────

fn xdr_hex<T: IntoVal<Env, Val>>(env: &Env, val: T) -> std::string::String {
    let xdr: Bytes = val.to_xdr(env);
    let mut out = std::string::String::with_capacity(xdr.len() as usize * 2);
    for byte in xdr.iter() {
        out.push_str(&std::format!("{:02x}", byte));
    }
    out
}

fn fixture_path(name: &str) -> std::string::String {
    std::format!(
        "{}/../../event-fixtures/{}.hex",
        env!("CARGO_MANIFEST_DIR"),
        name
    )
}

/// Same contract as `shared::event_fixtures_tests::assert_fixture`: a missing
/// or `PENDING` fixture is generated, `UPDATE_EVENT_FIXTURES=1` rewrites it,
/// otherwise the encoding must match byte for byte.
fn assert_fixture(name: &str, actual_hex: &str) {
    let path = fixture_path(name);
    let update_mode = std::env::var("UPDATE_EVENT_FIXTURES").unwrap_or_default() == "1";
    let baseline = std::fs::read_to_string(&path)
        .ok()
        .map(|c| std::string::String::from(c.trim()))
        .filter(|c| c != "PENDING");
    match baseline {
        Some(expected) if !update_mode => assert_eq!(
            actual_hex, expected,
            "\nVALIDATOR EVENT SERIALIZATION REGRESSION [{name}]: the XDR encoding changed. \
             If intentional, bump VALIDATOR_EVENT_SCHEMA_VERSION and regenerate with \
             UPDATE_EVENT_FIXTURES=1 cargo test -p bridge test_validator_accounting"
        ),
        _ => {
            std::fs::write(&path, actual_hex).expect("failed to write event fixture");
            std::eprintln!("FIXTURE_UPDATE: wrote {name}.hex");
        }
    }
}

/// Asserts the field names of an encoded struct, in wire order
/// (alphabetical for `contracttype` structs).
fn assert_field_names(env: &Env, val: Val, expected: &[&str]) {
    let map: Map<Symbol, Val> = Map::try_from_val(env, &val).unwrap();
    let mut want: Vec<Symbol> = Vec::new(env);
    for name in expected {
        want.push_back(Symbol::new(env, name));
    }
    assert_eq!(map.keys(), want);
}

fn fixture_env() -> (Env, Address) {
    let env = Env::default();
    // Fresh env: deterministic test address sequence.
    let validator = Address::generate(&env);
    (env, validator)
}

#[test]
fn fixture_evt_validator_reward_credited() {
    let (env, validator) = fixture_env();
    let evt = EvtValidatorRewardCredited {
        schema_version: VALIDATOR_EVENT_SCHEMA_VERSION,
        validator,
        amount: 1_000_000,
        reason: ValidatorRewardReason::Attestation,
        epoch: 42,
        balance_after: 3_000_000,
        total_rewarded: 4_000_000,
    };
    let val: Val = evt.clone().into_val(&env);
    assert_field_names(
        &env,
        val,
        &[
            "amount",
            "balance_after",
            "epoch",
            "reason",
            "schema_version",
            "total_rewarded",
            "validator",
        ],
    );
    // Reason categories are encoded as stable u32 discriminants.
    let map: Map<Symbol, Val> = Map::try_from_val(&env, &val).unwrap();
    let reason = map.get(Symbol::new(&env, "reason")).unwrap();
    assert_eq!(u32::try_from_val(&env, &reason).unwrap(), 2);

    assert_fixture("evt_bridge_validator_reward_credited", &xdr_hex(&env, evt));
}

#[test]
fn fixture_evt_validator_slashed() {
    let (env, validator) = fixture_env();
    let evt = EvtValidatorSlashed {
        schema_version: VALIDATOR_EVENT_SCHEMA_VERSION,
        validator,
        amount: 500_000,
        reason: ValidatorSlashReason::InvalidAttestation,
        epoch: 42,
        balance_after: 2_500_000,
        total_slashed: 500_000,
    };
    let val: Val = evt.clone().into_val(&env);
    assert_field_names(
        &env,
        val,
        &[
            "amount",
            "balance_after",
            "epoch",
            "reason",
            "schema_version",
            "total_slashed",
            "validator",
        ],
    );
    let map: Map<Symbol, Val> = Map::try_from_val(&env, &val).unwrap();
    let reason = map.get(Symbol::new(&env, "reason")).unwrap();
    assert_eq!(u32::try_from_val(&env, &reason).unwrap(), 2);

    assert_fixture("evt_bridge_validator_slashed", &xdr_hex(&env, evt));
}
