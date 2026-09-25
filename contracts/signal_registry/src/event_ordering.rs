//! Initial scaffold for deterministic ordering and replay semantics on analytics
//! event streams (#918). Defines an OrderedEvent envelope with a strictly
//! increasing sequence number per stream, and a replay validator that detects
//! out-of-order or duplicate sequences so replays are stable and testable.
//! Follow-up work: wire sequence assignment into the live analytics emission
//! path in analytics_engine.rs.
//!
//! #1067: adds versioned topic conventions so indexers can identify schema
//! changes without guessing from payload shape. Each event family carries a
//! `topic_version` and a namespaced topic symbol (`<family>_v<version>`).

use soroban_sdk::{contracttype, symbol_short, Symbol};

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

/// Event families that participate in topic version negotiation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[contracttype]
pub enum EventFamily {
    Analytics,
    Signal,
    Registry,
}

/// Current schema version emitted for each event family. Bump the relevant
/// constant when a family's payload schema changes so indexers can detect the
/// change from the topic alone.
pub const ANALYTICS_TOPIC_VERSION: u32 = 1;
pub const SIGNAL_TOPIC_VERSION: u32 = 1;
pub const REGISTRY_TOPIC_VERSION: u32 = 1;

/// Returns the schema version currently emitted for `family`.
pub fn topic_version(family: EventFamily) -> u32 {
    match family {
        EventFamily::Analytics => ANALYTICS_TOPIC_VERSION,
        EventFamily::Signal => SIGNAL_TOPIC_VERSION,
        EventFamily::Registry => REGISTRY_TOPIC_VERSION,
    }
}

/// Returns the versioned topic symbol for `family`, e.g. `analytics_v1`.
/// Indexers match on this symbol to select the correct decoder.
pub fn versioned_topic(family: EventFamily) -> Symbol {
    match family {
        EventFamily::Analytics => symbol_short!("analytics_v1"),
        EventFamily::Signal => symbol_short!("signal_v1"),
        EventFamily::Registry => symbol_short!("registry_v1"),
    }
}

/// Parses the trailing `_v<version>` suffix from a topic symbol. Returns `None`
/// when the symbol does not follow the versioned convention, letting consumers
/// fall back to legacy (unversioned) decoding.
pub fn parse_topic_version(topic: &Symbol) -> Option<u32> {
    let s = topic.to_string();
    let idx = s.rfind("_v")?;
    let suffix = &s[idx + 2..];
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<u32>().ok()
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
    fn versioned_topic_matches_declared_version() {
        assert_eq!(parse_topic_version(&versioned_topic(EventFamily::Analytics)), Some(ANALYTICS_TOPIC_VERSION));
        assert_eq!(parse_topic_version(&versioned_topic(EventFamily::Signal)), Some(SIGNAL_TOPIC_VERSION));
        assert_eq!(parse_topic_version(&versioned_topic(EventFamily::Registry)), Some(REGISTRY_TOPIC_VERSION));
    }

    #[test]
    fn legacy_unversioned_topic_is_distinguishable() {
        // Old consumers see no version suffix and fall back to legacy decoding.
        assert_eq!(parse_topic_version(&symbol_short!("analytics")), None);
        assert_eq!(parse_topic_version(&symbol_short!("signal")), None);
    }

    #[test]
    fn new_consumer_can_distinguish_versions() {
        let v1 = versioned_topic(EventFamily::Analytics);
        let v2 = symbol_short!("analytics_v2");
        assert_ne!(parse_topic_version(&v1), parse_topic_version(&v2));
        assert_eq!(parse_topic_version(&v2), Some(2));
    }
}
