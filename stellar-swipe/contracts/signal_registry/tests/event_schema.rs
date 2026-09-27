//! Representative emitted events validated against `docs/event_schema.json`
//! (issue #1221).
//!
//! Each flow below drives the real contract, then checks every event it
//! emitted for the names under test: topic count and literal topics must
//! match `topics_format`, and the body must be a tuple whose length and value
//! types match `body_fields`. A drift between the code and the schema that
//! clients and indexers consume fails here.

use serde_json::Value as Json;
use signal_registry::{
    RiskLevel, SignalAction, SignalCategory, SignalRegistry, SignalRegistryClient,
};
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger},
    vec, Address, Env, String, Symbol, TryFromVal, Val, Vec,
};

const SCHEMA: &str = include_str!("../../../../docs/event_schema.json");
const CONTRACT: &str = "signal_registry";
const T0: u64 = 1_000_000;

fn schema_entry(name: &str) -> Json {
    let schema: Json = serde_json::from_str(SCHEMA).expect("event schema is valid JSON");
    schema["events"]
        .as_array()
        .expect("events array")
        .iter()
        .find(|e| e["contract"] == CONTRACT && e["event_name"] == name)
        .unwrap_or_else(|| panic!("{CONTRACT}.{name} is missing from docs/event_schema.json"))
        .clone()
}

fn val_has_type(env: &Env, val: &Val, ty: &str) -> bool {
    match ty {
        "u32" => u32::try_from_val(env, val).is_ok(),
        "u64" => u64::try_from_val(env, val).is_ok(),
        "i128" => i128::try_from_val(env, val).is_ok(),
        "bool" => bool::try_from_val(env, val).is_ok(),
        "Address" => Address::try_from_val(env, val).is_ok(),
        "String" => String::try_from_val(env, val).is_ok(),
        "Symbol" => Symbol::try_from_val(env, val).is_ok(),
        "Option<u64>" => Option::<u64>::try_from_val(env, val).is_ok(),
        "Vec<Address>" => Vec::<Address>::try_from_val(env, val).is_ok(),
        "Vec<String>" => Vec::<String>::try_from_val(env, val).is_ok(),
        other => panic!("schema type {other} has no validator in this test"),
    }
}

/// Validate every event named `name` emitted by the last invocation and
/// return how many were checked.
fn check_emitted(env: &Env, contract_id: &Address, name: &str) -> usize {
    let entry = schema_entry(name);
    let topics_format = entry["topics_format"].as_array().unwrap();
    let body_fields = entry["body_fields"].as_array().unwrap();
    let name_sym = Symbol::new(env, name);
    let mut checked = 0;

    for (emitter, topics, data) in env.events().all().iter() {
        let is_target = topics.iter().any(|t| {
            Symbol::try_from_val(env, &t)
                .map(|s| s == name_sym)
                .unwrap_or(false)
        });
        if emitter != *contract_id || !is_target {
            continue;
        }
        checked += 1;

        assert_eq!(
            topics.len() as usize,
            topics_format.len(),
            "{name}: topic count differs from schema"
        );
        for (i, format) in topics_format.iter().enumerate() {
            let format = format.as_str().unwrap();
            let topic = topics.get(i as u32).unwrap();
            let expected = match format {
                "symbol:event_name" => Some(name),
                f => f.strip_prefix("symbol:"),
            };
            if let Some(literal) = expected {
                let actual = Symbol::try_from_val(env, &topic)
                    .unwrap_or_else(|_| panic!("{name}: topic {i} is not a symbol"));
                assert_eq!(actual, Symbol::new(env, literal), "{name}: topic {i}");
            }
        }

        let body = Vec::<Val>::try_from_val(env, &data)
            .unwrap_or_else(|_| panic!("{name}: body is not a tuple"));
        assert_eq!(
            body.len() as usize,
            body_fields.len(),
            "{name}: body arity differs from schema"
        );
        for (i, field) in body_fields.iter().enumerate() {
            let ty = field["type"].as_str().unwrap();
            assert!(
                val_has_type(env, &body.get(i as u32).unwrap(), ty),
                "{name}: body field {} is not {ty}",
                field["name"]
            );
        }
    }
    checked
}

fn setup() -> (Env, Address, SignalRegistryClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(T0);
    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, contract_id, client, admin)
}

fn create(env: &Env, client: &SignalRegistryClient, expiry: u64) -> u64 {
    let provider = Address::generate(env);
    client.stake_tokens(&provider, &1_000_000_000i128);
    client.create_signal(
        &provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &100_000,
        &String::from_str(env, "Test"),
        &expiry,
        &SignalCategory::SWING,
        &vec![env, String::from_str(env, "test")],
        &RiskLevel::Medium,
    )
}

#[test]
fn admin_transfer_proposed_matches_schema() {
    let (env, id, client, admin) = setup();
    client.propose_admin_transfer(&admin, &Address::generate(&env));
    assert_eq!(check_emitted(&env, &id, "admin_transfer_proposed"), 1);
}

#[test]
fn follow_gained_matches_schema() {
    let (env, id, client, _) = setup();
    client.follow_provider(&Address::generate(&env), &Address::generate(&env));
    assert_eq!(check_emitted(&env, &id, "follow_gained"), 1);
}

#[test]
fn signal_expired_matches_schema() {
    let (env, id, client, _) = setup();
    create(&env, &client, T0 + 10);
    create(&env, &client, T0 + 10);
    env.ledger().set_timestamp(T0 + 100);
    client.cleanup_expired_signals(&0);
    assert_eq!(check_emitted(&env, &id, "signal_expired"), 2);
}

#[test]
fn signals_pruned_matches_schema() {
    let (env, id, client, admin) = setup();
    create(&env, &client, T0 + 10);
    env.ledger().set_timestamp(T0 + 100);
    client.prune_expired_signals(&admin, &10);
    assert_eq!(check_emitted(&env, &id, "signals_pruned"), 1);
}

#[test]
fn signals_archived_matches_schema() {
    let (env, id, client, _) = setup();
    create(&env, &client, T0 + 10);
    env.ledger().set_timestamp(T0 + 100 + 30 * 24 * 60 * 60);
    client.archive_old_signals(&0);
    assert_eq!(check_emitted(&env, &id, "signals_archived"), 1);
    // The never-cleaned signal is announced as expired in the same call.
    assert_eq!(check_emitted(&env, &id, "signal_expired"), 1);
}
