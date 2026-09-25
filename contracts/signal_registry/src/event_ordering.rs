//! Initial scaffold for deterministic ordering and replay semantics on analytics
//! event streams (#918). Defines an OrderedEvent envelope with a strictly
//! increasing sequence number per stream, and a replay validator that detects
//! out-of-order or duplicate sequences so replays are stable and testable.
//! Follow-up work: wire sequence assignment into the live analytics emission
//! path in analytics_engine.rs.
//!
//! Also hosts the contract client compatibility fixtures (#1076): serialized
//! request, response, event, and error payloads that a representative Rust
//! client round-trips against the built WASM contracts. Fixture updates are
//! reviewed as interface changes.

use soroban_sdk::{contracttype, Bytes, Env, IntoVal, TryFromVal, Val};

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

/// Serialized request payload a representative Rust client sends to the
/// contract interface. Kept as a stable, versioned fixture so client and
/// contract deserialization stay in lockstep.
#[derive(Clone)]
#[contracttype]
pub struct ClientRequest {
    pub stream_id: u64,
    pub sequence: u64,
    pub payload_hash: u64,
}

/// Serialized response payload the contract returns for a successful call.
#[derive(Clone)]
#[contracttype]
pub struct ClientResponse {
    pub accepted: bool,
    pub sequence: u64,
}

/// Serialized error payload the contract returns for a documented failure.
#[derive(Clone)]
#[contracttype]
pub struct ClientError {
    pub code: u32,
    pub sequence: u64,
}

/// Documented failure codes surfaced to clients. These are part of the ABI and
/// must not be renumbered without a fixture update.
pub const ERR_OUT_OF_ORDER: u32 = 1;
pub const ERR_DUPLICATE_SEQUENCE: u32 = 2;

/// Maps a replay validation failure to the documented client error payload.
pub fn error_fixture(err: &ReplayError) -> ClientError {
    match err {
        ReplayError::OutOfOrder { got, .. } => ClientError { code: ERR_OUT_OF_ORDER, sequence: *got },
        ReplayError::DuplicateSequence(seq) => {
            ClientError { code: ERR_DUPLICATE_SEQUENCE, sequence: *seq }
        }
    }
}

/// Serializes a value into the contract's canonical `Bytes` encoding, the same
/// representation a client must be able to deserialize.
pub fn serialize_fixture<T: IntoVal<Env, Val>>(env: &Env, value: &T) -> Bytes {
    value.into_val(env).try_into_val(env).unwrap_or_else(|_| Bytes::new(env))
}

/// Deserializes a fixture payload back into `T`, mirroring the client-side
/// decode path so compatibility regressions surface in tests.
pub fn deserialize_fixture<T: TryFromVal<Env, Val>>(env: &Env, bytes: &Bytes) -> Option<T> {
    let val: Val = bytes.clone().into_val(env);
    T::try_from_val(env, &val).ok()
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::Env;

    fn ev(stream_id: u64, sequence: u64) -> OrderedEvent {
        OrderedEvent { stream_id, sequence, payload_hash: 0 }
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
    fn request_fixture_round_trips_through_serialization() {
        let env = Env::default();
        let request = ClientRequest { stream_id: 7, sequence: 1, payload_hash: 42 };
        let bytes = serialize_fixture(&env, &request);
        let decoded: Option<ClientRequest> = deserialize_fixture(&env, &bytes);
        let decoded = decoded.expect("request fixture must deserialize");
        assert_eq!(decoded.stream_id, request.stream_id);
        assert_eq!(decoded.sequence, request.sequence);
        assert_eq!(decoded.payload_hash, request.payload_hash);
    }

    #[test]
    fn response_fixture_round_trips_through_serialization() {
        let env = Env::default();
        let response = ClientResponse { accepted: true, sequence: 1 };
        let bytes = serialize_fixture(&env, &response);
        let decoded: Option<ClientResponse> = deserialize_fixture(&env, &bytes);
        let decoded = decoded.expect("response fixture must deserialize");
        assert!(decoded.accepted);
        assert_eq!(decoded.sequence, response.sequence);
    }

    #[test]
    fn event_fixture_round_trips_through_serialization() {
        let env = Env::default();
        let event = ev(3, 9);
        let bytes = serialize_fixture(&env, &event);
        let decoded: Option<OrderedEvent> = deserialize_fixture(&env, &bytes);
        let decoded = decoded.expect("event fixture must deserialize");
        assert_eq!(decoded.stream_id, event.stream_id);
        assert_eq!(decoded.sequence, event.sequence);
    }

    #[test]
    fn documented_failure_cases_map_to_error_fixtures() {
        let out_of_order = error_fixture(&ReplayError::OutOfOrder { expected: 3, got: 1 });
        assert_eq!(out_of_order.code, ERR_OUT_OF_ORDER);
        assert_eq!(out_of_order.sequence, 1);

        let duplicate = error_fixture(&ReplayError::DuplicateSequence(5));
        assert_eq!(duplicate.code, ERR_DUPLICATE_SEQUENCE);
        assert_eq!(duplicate.sequence, 5);
    }

    #[test]
    fn error_fixture_round_trips_through_serialization() {
        let env = Env::default();
        let error = error_fixture(&ReplayError::DuplicateSequence(5));
        let bytes = serialize_fixture(&env, &error);
        let decoded: Option<ClientError> = deserialize_fixture(&env, &bytes);
        let decoded = decoded.expect("error fixture must deserialize");
        assert_eq!(decoded.code, ERR_DUPLICATE_SEQUENCE);
        assert_eq!(decoded.sequence, 5);
    }
}
