//! Contract-level tests for #1377 (canonical event identity / dedup) and
//! #1378 (half-open time-window boundaries).

use crate::ingestion::{AnalyticsEvent, EventIdentity, IngestError, WindowAggregate};
use crate::query_cache::QueryType;
use crate::window::{BUCKET_SECONDS, MAX_WINDOW_BUCKETS};
use crate::{AnalyticsContract, AnalyticsContractClient, ProtocolSnapshot};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Env, Vec,
};

const H: u64 = BUCKET_SECONDS;

fn setup(env: &Env) -> AnalyticsContractClient<'_> {
    env.mock_all_auths();
    let id = env.register(AnalyticsContract, ());
    let client = AnalyticsContractClient::new(env, &id);
    client.initialize(&Address::generate(env));
    client
}

fn ident(ledger: u32, tx_index: u32, op_index: u32, event_index: u32) -> EventIdentity {
    EventIdentity {
        ledger,
        tx_index,
        op_index,
        event_index,
    }
}

fn ev(id: EventIdentity, timestamp: u64, volume: i128) -> AnalyticsEvent {
    AnalyticsEvent {
        id,
        timestamp,
        volume,
    }
}

fn totals(client: &AnalyticsContractClient, start: u64, end: u64) -> (u64, i128) {
    let w = client.get_window_aggregate(&start, &end);
    (w.event_count, w.volume)
}

// ── #1377 canonical event identity ──────────────────────────────────────────

#[test]
fn duplicate_identity_is_counted_once() {
    let env = Env::default();
    let c = setup(&env);
    let e = ev(ident(100, 0, 0, 0), 10, 500);

    assert!(c.ingest_event(&e));
    // Retry after e.g. a client timeout: same identity, same payload.
    assert!(!c.ingest_event(&e));
    assert!(!c.ingest_event(&e));

    assert_eq!(totals(&c, 0, H), (1, 500));
    assert!(c.is_event_ingested(&e.id));
}

#[test]
fn replayed_batch_does_not_change_aggregates() {
    let env = Env::default();
    let c = setup(&env);
    let batch = vec![
        &env,
        ev(ident(100, 0, 0, 0), 10, 1),
        ev(ident(100, 1, 0, 0), 20, 2),
        ev(ident(101, 0, 0, 0), 30, 4),
    ];

    assert_eq!(c.ingest_events(&batch), 3);
    assert_eq!(totals(&c, 0, H), (3, 7));

    // Full replay of the same range (e.g. indexer restarted from a checkpoint).
    assert_eq!(c.ingest_events(&batch), 0);
    assert_eq!(totals(&c, 0, H), (3, 7));

    // Overlapping replay: one old event plus one new one.
    let overlap = vec![&env, batch.get(2).unwrap(), ev(ident(102, 0, 0, 0), 40, 8)];
    assert_eq!(c.ingest_events(&overlap), 1);
    assert_eq!(totals(&c, 0, H), (4, 15));
}

#[test]
fn duplicates_within_one_batch_are_counted_once() {
    let env = Env::default();
    let c = setup(&env);
    let e = ev(ident(7, 3, 0, 1), 5, 100);
    assert_eq!(c.ingest_events(&vec![&env, e.clone(), e.clone(), e]), 1);
    assert_eq!(totals(&c, 0, H), (1, 100));
}

#[test]
fn distinct_same_ledger_events_each_count() {
    let env = Env::default();
    let c = setup(&env);
    // All in ledger 500 with the same timestamp and volume; only their
    // position within the ledger differs.
    let batch = vec![
        &env,
        ev(ident(500, 0, 0, 0), 60, 10), // tx 0, first event
        ev(ident(500, 0, 0, 1), 60, 10), // same tx/op, second event
        ev(ident(500, 0, 1, 0), 60, 10), // same tx, next operation
        ev(ident(500, 1, 0, 0), 60, 10), // next transaction
    ];
    assert_eq!(c.ingest_events(&batch), 4);
    assert_eq!(totals(&c, 0, H), (4, 40));
    for e in batch.iter() {
        assert!(c.is_event_ingested(&e.id));
    }
    assert!(!c.is_event_ingested(&ident(500, 2, 0, 0)));
}

#[test]
fn conflicting_payload_for_same_identity_is_rejected() {
    let env = Env::default();
    let c = setup(&env);
    let id = ident(9, 0, 0, 0);
    assert!(c.ingest_event(&ev(id.clone(), 10, 100)));

    assert_eq!(
        c.try_ingest_event(&ev(id.clone(), 10, 999)),
        Err(Ok(IngestError::EventIdentityConflict))
    );
    // A different timestamp would move it into another bucket — also a conflict.
    assert_eq!(
        c.try_ingest_event(&ev(id, 2 * H, 100)),
        Err(Ok(IngestError::EventIdentityConflict))
    );
    assert_eq!(totals(&c, 0, 3 * H), (1, 100));
}

#[test]
fn failing_batch_is_atomic() {
    let env = Env::default();
    let c = setup(&env);
    let batch = vec![
        &env,
        ev(ident(1, 0, 0, 0), 10, 5),
        ev(ident(1, 1, 0, 0), 10, -1), // invalid
    ];
    assert_eq!(
        c.try_ingest_events(&batch),
        Err(Ok(IngestError::InvalidVolume))
    );
    // First event was not recorded either, so a corrected retry counts it.
    assert!(!c.is_event_ingested(&ident(1, 0, 0, 0)));
    assert_eq!(totals(&c, 0, H), (0, 0));
}

#[test]
fn ingest_requires_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(AnalyticsContract, ());
    let c = AnalyticsContractClient::new(&env, &id);
    assert_eq!(
        c.try_ingest_event(&ev(ident(1, 0, 0, 0), 0, 1)),
        Err(Ok(IngestError::NotInitialized))
    );
}

#[test]
fn bucket_volume_overflow_is_reported() {
    let env = Env::default();
    let c = setup(&env);
    assert!(c.ingest_event(&ev(ident(1, 0, 0, 0), 0, i128::MAX)));
    assert_eq!(
        c.try_ingest_event(&ev(ident(1, 1, 0, 0), 1, 1)),
        Err(Ok(IngestError::AggregateOverflow))
    );
    assert_eq!(totals(&c, 0, H), (1, i128::MAX));
    assert!(!c.is_event_ingested(&ident(1, 1, 0, 0)));
}

// ── #1378 half-open window boundaries ───────────────────────────────────────

fn ingest_at(env: &Env, c: &AnalyticsContractClient, timestamps: &[u64]) {
    let mut batch: Vec<AnalyticsEvent> = Vec::new(env);
    for (i, ts) in timestamps.iter().enumerate() {
        batch.push_back(ev(ident(1_000, i as u32, 0, 0), *ts, 1));
    }
    c.ingest_events(&batch);
}

#[test]
fn boundary_event_belongs_to_the_later_window_only() {
    let env = Env::default();
    let c = setup(&env);
    ingest_at(&env, &c, &[H]); // exactly on the boundary between [0,H) and [H,2H)

    assert_eq!(totals(&c, 0, H), (0, 0));
    assert_eq!(totals(&c, H, 2 * H), (1, 1));
}

#[test]
fn adjacent_windows_have_no_gaps_or_double_counting() {
    let env = Env::default();
    let c = setup(&env);
    // Events at both edges of each bucket plus one past the last window end.
    ingest_at(&env, &c, &[0, H - 1, H, 2 * H - 1, 2 * H, 3 * H - 1, 3 * H]);

    let a = totals(&c, 0, H);
    let b = totals(&c, H, 2 * H);
    let d = totals(&c, 2 * H, 3 * H);
    assert_eq!((a.0, b.0, d.0), (2, 2, 2));

    // The union equals the sum of the parts (no gaps, no overlap) and excludes
    // the event at the exclusive end `3H`.
    let whole = totals(&c, 0, 3 * H);
    assert_eq!(whole.0, a.0 + b.0 + d.0);
    assert_eq!(whole, (6, 6));
    assert_eq!(totals(&c, 0, 4 * H), (7, 7));

    // Splitting at a different point gives the same total.
    let left = totals(&c, 0, 2 * H);
    let right = totals(&c, 2 * H, 3 * H);
    assert_eq!(left.0 + right.0, whole.0);
}

#[test]
fn empty_window_is_zero() {
    let env = Env::default();
    let c = setup(&env);
    ingest_at(&env, &c, &[H]);
    assert_eq!(
        c.get_window_aggregate(&H, &H),
        WindowAggregate {
            start: H,
            end: H,
            event_count: 0,
            volume: 0,
        }
    );
}

#[test]
fn invalid_windows_are_rejected() {
    let env = Env::default();
    let c = setup(&env);
    assert_eq!(
        c.try_get_window_aggregate(&(2 * H), &H),
        Err(Ok(IngestError::InvalidWindow))
    );
    assert_eq!(
        c.try_get_window_aggregate(&1, &H),
        Err(Ok(IngestError::UnalignedWindow))
    );
    assert_eq!(
        c.try_get_window_aggregate(&0, &(H + 1)),
        Err(Ok(IngestError::UnalignedWindow))
    );
    // Exactly the maximum span is allowed; one more bucket is not.
    assert!(c
        .try_get_window_aggregate(&0, &(MAX_WINDOW_BUCKETS * H))
        .is_ok());
    assert_eq!(
        c.try_get_window_aggregate(&0, &((MAX_WINDOW_BUCKETS + 1) * H)),
        Err(Ok(IngestError::WindowTooLarge))
    );
}

#[test]
fn window_aggregates_are_deterministic_across_ingestion_order() {
    let env_a = Env::default();
    let a = setup(&env_a);
    let env_b = Env::default();
    let b = setup(&env_b);

    let events = [
        ev(ident(1, 0, 0, 0), 0, 3),
        ev(ident(1, 1, 0, 0), H - 1, 5),
        ev(ident(2, 0, 0, 0), H, 7),
    ];
    for e in events.iter() {
        a.ingest_event(e);
    }
    for e in events.iter().rev() {
        b.ingest_event(e);
    }
    for (s, e) in [(0, H), (H, 2 * H), (0, 2 * H)] {
        assert_eq!(totals(&a, s, e), totals(&b, s, e));
    }
}

#[test]
fn cache_ttl_follows_half_open_convention() {
    let env = Env::default();
    let c = setup(&env);
    c.set_cache_ttl(&QueryType::TotalVolume, &10u64);
    let snap = |volume: i128| ProtocolSnapshot {
        total_signals: 0,
        active_signals: 0,
        total_providers: 0,
        total_executions: 0,
        total_volume: volume,
        avg_success_rate_bps: 0,
        timestamp: 0,
    };

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    c.update_snapshot(&snap(1));
    assert_eq!(c.get_total_volume_cached(), 1);
    c.update_snapshot(&snap(2));

    // Last instant inside [1000, 1010): still a hit.
    env.ledger().with_mut(|l| l.timestamp = 1_009);
    assert_eq!(c.get_total_volume_cached(), 1);
    // Exactly at the exclusive end: a miss.
    env.ledger().with_mut(|l| l.timestamp = 1_010);
    assert_eq!(c.get_total_volume_cached(), 2);
}

#[test]
fn weekly_report_follows_half_open_convention() {
    use stellar_swipe_common::SECONDS_PER_WEEK;
    let env = Env::default();
    let c = setup(&env);
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    c.emit_weekly_health_report();

    // Last instant inside [1000, 1000 + week): rejected.
    env.ledger()
        .with_mut(|l| l.timestamp = 1_000 + SECONDS_PER_WEEK - 1);
    assert!(c.try_emit_weekly_health_report().is_err());
    // Exactly at the exclusive end: allowed.
    env.ledger()
        .with_mut(|l| l.timestamp = 1_000 + SECONDS_PER_WEEK);
    c.emit_weekly_health_report();
    assert_eq!(c.get_last_report_time(), 1_000 + SECONDS_PER_WEEK);
}
