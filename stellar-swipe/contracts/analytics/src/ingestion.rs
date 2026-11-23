//! Idempotent analytics event ingestion (Issue #1377) bucketed by the
//! half-open window convention in [`crate::window`] (Issue #1378).
//!
//! # Canonical event identity
//!
//! A contract event is identified by its position in the ledger history:
//! `(ledger, tx_index, op_index, event_index)` — see [`EventIdentity`]. This is
//! the same coordinate system Stellar RPC uses for event ids (a TOID of ledger,
//! transaction and operation, plus the event's index within the operation), so
//! an indexer can derive it deterministically from any source and every
//! re-delivery of the same event maps to the same identity.
//!
//! Payload fields (timestamp, volume) are deliberately *not* part of the
//! identity: a replay that reports the same event with different data is a
//! conflict, not a new event.
//!
//! # Deduplication
//!
//! The first ingestion of an identity records the event and adds it to its
//! bucket's aggregate. Re-ingesting the same identity with an identical payload
//! (replays, retries after a timeout) is a no-op that returns `false`, so
//! aggregates are never counted twice. Re-ingesting it with a *different*
//! payload fails with [`IngestError::EventIdentityConflict`]. Distinct events
//! in the same ledger — or the same transaction — have distinct identities and
//! are each counted.
//!
//! Batch ingestion is atomic: an error anywhere in the batch reverts the whole
//! call, so a partially applied batch is never observable and a retry of the
//! full batch is safe.

use soroban_sdk::{contracterror, contracttype, Env, Vec};

use crate::window::{self, BUCKET_SECONDS, MAX_WINDOW_BUCKETS};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum IngestError {
    /// Contract has not been initialized (no admin).
    NotInitialized = 1,
    /// Event volume must be non-negative.
    InvalidVolume = 2,
    /// Identity already ingested with a different payload.
    EventIdentityConflict = 3,
    /// Aggregate count or volume overflowed.
    AggregateOverflow = 4,
    /// Window `end` precedes `start`.
    InvalidWindow = 5,
    /// Window bound is not a multiple of `BUCKET_SECONDS`.
    UnalignedWindow = 6,
    /// Window spans more than `MAX_WINDOW_BUCKETS` buckets.
    WindowTooLarge = 7,
}

/// Canonical identity of one contract event.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventIdentity {
    /// Ledger sequence the event was emitted in.
    pub ledger: u32,
    /// Application order of the transaction within the ledger.
    pub tx_index: u32,
    /// Index of the operation within the transaction.
    pub op_index: u32,
    /// Index of the event within the operation's emitted events.
    pub event_index: u32,
}

/// One analytics input derived from a contract event.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalyticsEvent {
    pub id: EventIdentity,
    /// Ledger close time of the event; selects its bucket.
    pub timestamp: u64,
    /// Volume contributed by the event (base units, `>= 0`).
    pub volume: i128,
}

/// Aggregate over one bucket or one `[start, end)` window.
#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Aggregate {
    pub event_count: u64,
    pub volume: i128,
}

/// Result of a window query: the half-open window `[start, end)` and its totals.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowAggregate {
    /// Inclusive start (seconds).
    pub start: u64,
    /// Exclusive end (seconds).
    pub end: u64,
    pub event_count: u64,
    pub volume: i128,
}

#[contracttype]
#[derive(Clone)]
enum IngestKey {
    /// Payload recorded for an ingested identity.
    Seen(EventIdentity),
    /// Aggregate for bucket index `k` = `[k * BUCKET_SECONDS, (k + 1) * BUCKET_SECONDS)`.
    Bucket(u64),
}

pub fn is_ingested(env: &Env, id: &EventIdentity) -> bool {
    env.storage().persistent().has(&IngestKey::Seen(id.clone()))
}

fn bucket(env: &Env, k: u64) -> Aggregate {
    env.storage()
        .persistent()
        .get(&IngestKey::Bucket(k))
        .unwrap_or_default()
}

/// Ingest one event. Returns `true` if it was counted, `false` if it was an
/// exact duplicate of an already-ingested event (no state change).
pub fn ingest(env: &Env, event: &AnalyticsEvent) -> Result<bool, IngestError> {
    if event.volume < 0 {
        return Err(IngestError::InvalidVolume);
    }
    let seen_key = IngestKey::Seen(event.id.clone());
    if let Some(prev) = env
        .storage()
        .persistent()
        .get::<_, AnalyticsEvent>(&seen_key)
    {
        return if prev == *event {
            Ok(false)
        } else {
            Err(IngestError::EventIdentityConflict)
        };
    }

    let k = window::bucket_of(event.timestamp);
    let mut agg = bucket(env, k);
    agg.event_count = agg
        .event_count
        .checked_add(1)
        .ok_or(IngestError::AggregateOverflow)?;
    agg.volume = agg
        .volume
        .checked_add(event.volume)
        .ok_or(IngestError::AggregateOverflow)?;

    env.storage().persistent().set(&IngestKey::Bucket(k), &agg);
    env.storage().persistent().set(&seen_key, event);
    Ok(true)
}

/// Ingest a batch atomically. Returns the number of newly counted events.
/// Duplicates within the batch or against earlier ingestions are skipped.
pub fn ingest_batch(env: &Env, events: &Vec<AnalyticsEvent>) -> Result<u32, IngestError> {
    let mut counted = 0u32;
    for e in events.iter() {
        if ingest(env, &e)? {
            counted += 1;
        }
    }
    Ok(counted)
}

/// Validate a `[start, end)` query window and return its bucket range `[first, last)`.
fn window_buckets(start: u64, end: u64) -> Result<(u64, u64), IngestError> {
    if end < start {
        return Err(IngestError::InvalidWindow);
    }
    if start % BUCKET_SECONDS != 0 || end % BUCKET_SECONDS != 0 {
        return Err(IngestError::UnalignedWindow);
    }
    let first = start / BUCKET_SECONDS;
    let last = end / BUCKET_SECONDS;
    if last - first > MAX_WINDOW_BUCKETS {
        return Err(IngestError::WindowTooLarge);
    }
    Ok((first, last))
}

/// Totals for events with `start <= timestamp < end` (half-open, see
/// [`crate::window`]). Both bounds must be multiples of `BUCKET_SECONDS`.
pub fn window_aggregate(env: &Env, start: u64, end: u64) -> Result<WindowAggregate, IngestError> {
    let (first, last) = window_buckets(start, end)?;
    let mut event_count: u64 = 0;
    let mut volume: i128 = 0;
    for k in first..last {
        let b = bucket(env, k);
        event_count = event_count
            .checked_add(b.event_count)
            .ok_or(IngestError::AggregateOverflow)?;
        volume = volume
            .checked_add(b.volume)
            .ok_or(IngestError::AggregateOverflow)?;
    }
    Ok(WindowAggregate {
        start,
        end,
        event_count,
        volume,
    })
}
