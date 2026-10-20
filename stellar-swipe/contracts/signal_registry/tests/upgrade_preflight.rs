//! Read-only upgrade preflight (issue #1223).
//!
//! `upgrade_preflight` must report the current version, whether the target
//! version is accepted, and the migration state, without writing storage,
//! emitting events or requiring authorization.

use signal_registry::{
    MigrationSnapshot, MigrationVerification, RiskLevel, SignalAction, SignalCategory,
    SignalRegistry, SignalRegistryClient, SignalStatus, SignalV1, StorageKey, UpgradePreflight,
    UpgradeReadiness,
};
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, Env, Map, String, Vec,
};

/// Version `initialize` stamps (`shared::version::SIGNAL_REGISTRY_VERSION`).
const CURRENT: u32 = 2;

fn setup() -> (Env, Address, SignalRegistryClient<'static>, Address) {
    let env = Env::default();
    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);
    (env, contract_id, client, admin)
}

fn v1_signal(env: &Env, id: u64) -> SignalV1 {
    SignalV1 {
        id,
        provider: Address::generate(env),
        asset_pair: String::from_str(env, "XLM/USDC"),
        action: SignalAction::Buy,
        price: 100_000,
        rationale: String::from_str(env, "legacy"),
        timestamp: 0,
        expiry: 10_000,
        status: SignalStatus::Active,
        executions: 0,
        successful_executions: 0,
        total_volume: 5,
        total_roi: 0,
        category: SignalCategory::SWING,
        tags: Vec::new(env),
        risk_level: RiskLevel::Medium,
        is_collaborative: false,
    }
}

/// Put the contract into the pre-migration v1 layout with `n` legacy rows.
fn seed_v1(env: &Env, contract_id: &Address, n: u64) {
    env.as_contract(contract_id, || {
        let mut v1 = Map::new(env);
        for id in 1..=n {
            v1.set(id, v1_signal(env, id));
        }
        let storage = env.storage().instance();
        storage.set(&StorageKey::SignalsV1, &v1);
        storage.set(&StorageKey::SignalCounter, &n);
        storage.set(&StorageKey::SchemaVersion, &1u32);
    });
}

/// Calls `upgrade_preflight` and asserts it wrote nothing, emitted nothing
/// and needed no authorization.
fn preflight(env: &Env, client: &SignalRegistryClient, target: u32) -> UpgradePreflight {
    let before = env.to_ledger_snapshot().ledger_entries;
    env.set_auths(&[]);
    let report = client.upgrade_preflight(&target);
    assert!(env.auths().is_empty(), "preflight must not require auth");
    assert_eq!(
        env.events().all().len(),
        0,
        "preflight must not emit events"
    );
    assert!(
        env.to_ledger_snapshot().ledger_entries == before,
        "preflight must not write storage"
    );
    env.mock_all_auths();
    report
}

#[test]
fn current_version_reports_current() {
    let (env, _, client, _) = setup();
    let report = preflight(&env, &client, CURRENT);
    assert_eq!(
        report,
        UpgradePreflight {
            readiness: UpgradeReadiness::Current,
            current_version: CURRENT,
            target_version: CURRENT,
            schema_version: 2,
            supported_schema_version: 2,
            pending_v1_records: 0,
            migration_cursor: 1,
            migration_verified: None,
        }
    );
}

#[test]
fn newer_target_reports_upgradeable() {
    let (env, _, client, _) = setup();
    let report = preflight(&env, &client, CURRENT + 1);
    assert_eq!(report.readiness, UpgradeReadiness::Upgradeable);
    assert_eq!(report.current_version, CURRENT);
    assert_eq!(report.target_version, CURRENT + 1);
}

#[test]
fn older_target_reports_incompatible_version() {
    let (env, _, client, _) = setup();
    let report = preflight(&env, &client, CURRENT - 1);
    assert_eq!(report.readiness, UpgradeReadiness::IncompatibleVersion);
}

#[test]
fn unknown_schema_reports_incompatible_schema() {
    let (env, contract_id, client, _) = setup();
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&StorageKey::SchemaVersion, &99u32);
    });
    let report = preflight(&env, &client, CURRENT + 1);
    assert_eq!(report.readiness, UpgradeReadiness::IncompatibleSchema);
    assert_eq!(report.schema_version, 99);
}

#[test]
fn pending_v1_rows_report_migration_required_until_migrated() {
    let (env, contract_id, client, admin) = setup();
    seed_v1(&env, &contract_id, 3);

    let report = preflight(&env, &client, CURRENT + 1);
    assert_eq!(report.readiness, UpgradeReadiness::MigrationRequired);
    assert_eq!(report.schema_version, 1);
    assert_eq!(report.pending_v1_records, 3);
    // A migration is required even when no version change is requested.
    assert_eq!(
        preflight(&env, &client, CURRENT).readiness,
        UpgradeReadiness::MigrationRequired
    );

    // Partial migration: still required, cursor advanced.
    client.migrate_signals_v1_to_v2(&admin, &2);
    let report = preflight(&env, &client, CURRENT + 1);
    assert_eq!(report.readiness, UpgradeReadiness::MigrationRequired);
    assert_eq!(report.pending_v1_records, 1);
    assert_eq!(report.migration_cursor, 3);

    client.migrate_signals_v1_to_v2(&admin, &2);
    let report = preflight(&env, &client, CURRENT + 1);
    assert_eq!(report.readiness, UpgradeReadiness::Upgradeable);
    assert_eq!(report.schema_version, 2);
    assert_eq!(report.pending_v1_records, 0);
    assert_eq!(report.migration_verified, Some(true));

    // The actual upgrade guard agrees with the preflight on the version.
    assert_eq!(client.get_contract_version(), report.current_version);
}

#[test]
fn failed_migration_verification_blocks_upgrade() {
    let (env, contract_id, client, _) = setup();
    env.as_contract(&contract_id, || {
        let snap = |record_count| MigrationSnapshot {
            record_count,
            total_volume_sum: 0,
        };
        env.storage().instance().set(
            &StorageKey::MigrationVerification,
            &MigrationVerification {
                verified: false,
                pre: snap(3),
                post: snap(2),
            },
        );
    });
    let report = preflight(&env, &client, CURRENT + 1);
    assert_eq!(report.readiness, UpgradeReadiness::MigrationUnverified);
    assert_eq!(report.migration_verified, Some(false));
}

#[test]
fn preflight_works_on_an_uninitialized_contract() {
    let env = Env::default();
    let contract_id = env.register(SignalRegistry, ());
    let client = SignalRegistryClient::new(&env, &contract_id);
    // Nothing stored: version defaults to 1 and schema to the legacy v1.
    let report = preflight(&env, &client, 2);
    assert_eq!(report.current_version, 1);
    assert_eq!(report.schema_version, 1);
    assert_eq!(report.readiness, UpgradeReadiness::Upgradeable);
}
