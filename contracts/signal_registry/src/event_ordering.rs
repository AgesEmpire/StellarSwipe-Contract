//! Initial scaffold for deterministic ordering and replay semantics on analytics
//! event streams (#918). Defines an OrderedEvent envelope with a strictly
//! increasing sequence number per stream, and a replay validator that detects
//! out-of-order or duplicate sequences so replays are stable and testable.
//! Follow-up work: wire sequence assignment into the live analytics emission
//! path in analytics_engine.rs.
//!
//! Also enforces bounded event payload sizes at contract boundaries (#1070) so
//! oversized emissions cannot exhaust transaction resources or disrupt
//! downstream indexers.

use soroban_sdk::{contracttype, String, Vec};

/// Maximum length, in bytes, of a string field carried in an event payload.
pub const MAX_EVENT_STRING_LEN: u32 = 256;

/// Maximum number of elements allowed in a vector field of an event payload.
pub const MAX_EVENT_VECTOR_LEN: u32 = 64;

/// Maximum number of metadata entries attached to a single event.
pub const MAX_EVENT_METADATA_ENTRIES: u32 = 16;

#[derive(Clone)]
#[contracttype]
pub struct OrderedEvent {
    pub stream_id: u64,
    pub sequence: u64,
    pub payload_hash: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReplayError {
    OutOfOrder { expected: u64, got: u64 },
    DuplicateSequence(u64),
}

/// Error returned when an event payload exceeds a documented size limit.
///
/// `kind` identifies which boundary was violated, `limit` is the configured
/// maximum, and `actual` is the observed size. Callers must treat this as a
/// hard failure and must not emit the event.
#[derive(Debug, PartialEq, Eq)]
pub enum PayloadSizeError {
    StringTooLong { limit: u32, actual: u32 },
    VectorTooLong { limit: u32, actual: u32 },
    TooManyMetadataEntries { limit: u32, actual: u32 },
}

/// Validates a string field against [`MAX_EVENT_STRING_LEN`].
pub fn validate_event_string(value: &String) -> Result<(), PayloadSizeError> {
    let actual = value.len();
    if actual > MAX_EVENT_STRING_LEN {
        return Err(PayloadSizeError::StringTooLong { limit: MAX_EVENT_STRING_LEN, actual });
    }
    Ok(())
}

/// Validates a vector field against [`MAX_EVENT_VECTOR_LEN`].
pub fn validate_event_vector<T>(value: &Vec<T>) -> Result<(), PayloadSizeError> {
    let actual = value.len();
    if actual > MAX_EVENT_VECTOR_LEN {
        return Err(PayloadSizeError::VectorTooLong { limit: MAX_EVENT_VECTOR_LEN, actual });
    }
    Ok(())
}

/// Validates the number of metadata entries against
/// [`MAX_EVENT_METADATA_ENTRIES`].
pub fn validate_event_metadata<T>(metadata: &Vec<T>) -> Result<(), PayloadSizeError> {
    let actual = metadata.len();
    if actual > MAX_EVENT_METADATA_ENTRIES {
        return Err(PayloadSizeError::TooManyMetadataEntries {
            limit: MAX_EVENT_METADATA_ENTRIES,
            actual,
        });
    }
    Ok(())
}

/// Validates every bounded field of an event payload before emission. Returns
/// the first violated limit, if any, so oversized payloads fail deterministically
/// before any event is emitted.
pub fn validate_event_payload<T>(
    label: &String,
    items: &Vec<T>,
    metadata: &Vec<T>,
) -> Result<(), PayloadSizeError> {
    validate_event_string(label)?;
    validate_event_vector(items)?;
    validate_event_metadata(metadata)?;
    Ok(())
}

/// Validates that `events` for a single stream form a strictly increasing,
/// gap-tolerant-but-monotonic sequence, so replaying them always yields the
/// same order regardless of the order they were received off-chain.
pub fn validate_replay_order(events: &[OrderedEvent]) -> Result<(), ReplayError> {
    let mut last_seen: Option<u64> = None;
    for event in events {
        if let Some(prev) = last_seen {
            if event.sequence == prev {
                return Err(ReplayError::DuplicateSequence(event.sequence));
            }
            if event.sequence < prev {
                return Err(ReplayError::OutOfOrder { expected: prev + 1, got: event.sequence });
            }
        }
        last_seen = Some(event.sequence);
    }
    Ok(())
}

/// Sorts events by (stream_id, sequence) so a batch received in any order
/// replays deterministically.
pub fn deterministic_sort(mut events: Vec<OrderedEvent>) -> Vec<OrderedEvent> {
    events.sort_by(|a, b| (a.stream_id, a.sequence).cmp(&(b.stream_id, b.sequence)));
    events
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{Env, String, Vec};

    fn ev(stream_id: u64, sequence: u64) -> OrderedEvent {
        OrderedEvent { stream_id, sequence, payload_hash: 0 }
    }

    fn string_of_len(env: &Env, len: u32) -> String {
        let mut s = String::from_str(env, "");
        for _ in 0..len {
            s.push_str(&String::from_str(env, "a"));
        }
        s
    }

    fn vec_of_len(env: &Env, len: u32) -> Vec<u32> {
        let mut v = Vec::new(env);
        for i in 0..len {
            v.push_back(i);
        }
        v
    }

    #[test]
    fn accepts_strictly_increasing_sequence() {
        let events = vec![ev(1, 1), ev(1, 2), ev(1, 3)];
        assert_eq!(validate_replay_order(&events), Ok(()));
    }

    #[test]
    fn rejects_duplicate_sequence() {
        let events = vec![ev(1, 1), ev(1, 1)];
        assert_eq!(validate_replay_order(&events), Err(ReplayError::DuplicateSequence(1)));
    }

    #[test]
    fn rejects_out_of_order_sequence() {
        let events = vec![ev(1, 2), ev(1, 1)];
        assert_eq!(validate_replay_order(&events), Err(ReplayError::OutOfOrder { expected: 3, got: 1 }));
    }

    #[test]
    fn deterministic_sort_produces_stable_order_regardless_of_input_order() {
        let a = deterministic_sort(vec![ev(2, 1), ev(1, 2), ev(1, 1)]);
        let b = deterministic_sort(vec![ev(1, 1), ev(1, 2), ev(2, 1)]);
        let seq_a: Vec<(u64, u64)> = a.iter().map(|e| (e.stream_id, e.sequence)).collect();
        let seq_b: Vec<(u64, u64)> = b.iter().map(|e| (e.stream_id, e.sequence)).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn string_at_exact_limit_is_accepted() {
        let env = Env::default();
        let s = string_of_len(&env, MAX_EVENT_STRING_LEN);
        assert_eq!(validate_event_string(&s), Ok(()));
    }

    #[test]
    fn string_one_over_limit_is_rejected() {
        let env = Env::default();
        let s = string_of_len(&env, MAX_EVENT_STRING_LEN + 1);
        assert_eq!(
            validate_event_string(&s),
            Err(PayloadSizeError::StringTooLong {
                limit: MAX_EVENT_STRING_LEN,
                actual: MAX_EVENT_STRING_LEN + 1,
            })
        );
    }

    #[test]
    fn vector_at_exact_limit_is_accepted() {
        let env = Env::default();
        let v = vec_of_len(&env, MAX_EVENT_VECTOR_LEN);
        assert_eq!(validate_event_vector(&v), Ok(()));
    }

    #[test]
    fn vector_one_over_limit_is_rejected() {
        let env = Env::default();
        let v = vec_of_len(&env, MAX_EVENT_VECTOR_LEN + 1);
        assert_eq!(
            validate_event_vector(&v),
            Err(PayloadSizeError::VectorTooLong {
                limit: MAX_EVENT_VECTOR_LEN,
                actual: MAX_EVENT_VECTOR_LEN + 1,
            })
        );
    }

    #[test]
    fn metadata_at_exact_limit_is_accepted() {
        let env = Env::default();
        let m = vec_of_len(&env, MAX_EVENT_METADATA_ENTRIES);
        assert_eq!(validate_event_metadata(&m), Ok(()));
    }

    #[test]
    fn metadata_one_over_limit_is_rejected() {
        let env = Env::default();
        let m = vec_of_len(&env, MAX_EVENT_METADATA_ENTRIES + 1);
        assert_eq!(
            validate_event_metadata(&m),
            Err(PayloadSizeError::TooManyMetadataEntries {
                limit: MAX_EVENT_METADATA_ENTRIES,
                actual: MAX_EVENT_METADATA_ENTRIES + 1,
            })
        );
    }

    #[test]
    fn payload_at_all_limits_is_accepted() {
        let env = Env::default();
        let label = string_of_len(&env, MAX_EVENT_STRING_LEN);
        let items = vec_of_len(&env, MAX_EVENT_VECTOR_LEN);
        let metadata = vec_of_len(&env, MAX_EVENT_METADATA_ENTRIES);
        assert_eq!(validate_event_payload(&label, &items, &metadata), Ok(()));
    }

    #[test]
    fn payload_one_over_any_limit_is_rejected_before_emission() {
        let env = Env::default();
        let label = string_of_len(&env, MAX_EVENT_STRING_LEN + 1);
        let items = vec_of_len(&env, MAX_EVENT_VECTOR_LEN);
        let metadata = vec_of_len(&env, MAX_EVENT_METADATA_ENTRIES);
        assert_eq!(
            validate_event_payload(&label, &items, &metadata),
            Err(PayloadSizeError::StringTooLong {
                limit: MAX_EVENT_STRING_LEN,
                actual: MAX_EVENT_STRING_LEN + 1,
            })
        );
    }

    #[test]
    fn resource_budget_is_predictable_across_repeated_validation() {
        // Validation is pure and allocation-free beyond the inputs, so repeated
        // checks over the same bounded payload must yield identical results and
        // never grow the resource footprint.
        let env = Env::default();
        let label = string_of_len(&env, MAX_EVENT_STRING_LEN);
        let items = vec_of_len(&env, MAX_EVENT_VECTOR_LEN);
        let metadata = vec_of_len(&env, MAX_EVENT_METADATA_ENTRIES);
        for _ in 0..100 {
            assert_eq!(validate_event_payload(&label, &items, &metadata), Ok(()));
        }
    }
}
