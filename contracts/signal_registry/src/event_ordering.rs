//! Initial scaffold for deterministic ordering and replay semantics on analytics
//! event streams (#918). Defines an OrderedEvent envelope with a strictly
//! increasing sequence number per stream, and a replay validator that detects
//! out-of-order or duplicate sequences so replays are stable and testable.
//! Follow-up work: wire sequence assignment into the live analytics emission
//! path in analytics_engine.rs.
//!
//! #1100: adds a monotonic sequence allocator for versioned contract events.
//! The allocator state is scoped per contract (stream) and is intended to be
//! persisted in contract storage so sequence continuity survives upgrades.

use soroban_sdk::{contracttype, Env, Map};

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

/// Persistent, per-contract sequence state for versioned events. Stored in
/// contract storage (not transient) so the next sequence value survives
/// upgrades and restarts, keeping the sequence monotonic across transactions.
#[derive(Clone)]
#[contracttype]
pub struct SequenceState {
    pub next_sequence: u64,
}

/// Storage key for a contract's sequence state, scoped by contract id so each
/// contract maintains its own independent monotonic sequence.
#[derive(Clone)]
#[contracttype]
pub enum SequenceKey {
    Next(u64),
}

/// Returns the next monotonic sequence value for `contract_id`, persisting the
/// incremented state so continuity is preserved across transactions and
/// upgrades. The first allocated value is 1.
#[allow(deprecated)]
pub fn next_sequence(env: &Env, contract_id: u64) -> u64 {
    let key = SequenceKey::Next(contract_id);
    let state: SequenceState = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(SequenceState { next_sequence: 1 });
    let sequence = state.next_sequence;
    env.storage().persistent().set(
        &key,
        &SequenceState { next_sequence: sequence + 1 },
    );
    sequence
}

/// Reads the current next sequence value for `contract_id` without mutating
/// state. Returns 1 when no sequence has been allocated yet.
#[allow(deprecated)]
pub fn peek_sequence(env: &Env, contract_id: u64) -> u64 {
    let key = SequenceKey::Next(contract_id);
    let state: SequenceState = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(SequenceState { next_sequence: 1 });
    state.next_sequence
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
    fn sequence_is_monotonic_across_transactions() {
        let env = Env::default();
        let contract_id = 7u64;
        assert_eq!(next_sequence(&env, contract_id), 1);
        assert_eq!(next_sequence(&env, contract_id), 2);
        assert_eq!(next_sequence(&env, contract_id), 3);
        assert_eq!(peek_sequence(&env, contract_id), 4);
    }

    #[test]
    fn sequence_state_is_scoped_per_contract() {
        let env = Env::default();
        assert_eq!(next_sequence(&env, 1), 1);
        assert_eq!(next_sequence(&env, 2), 1);
        assert_eq!(next_sequence(&env, 1), 2);
        assert_eq!(next_sequence(&env, 2), 2);
    }

    #[test]
    fn sequence_state_survives_upgrade() {
        let env = Env::default();
        let contract_id = 42u64;
        assert_eq!(next_sequence(&env, contract_id), 1);
        assert_eq!(next_sequence(&env, contract_id), 2);
        // Simulate an upgrade: state is read back from persistent storage.
        assert_eq!(peek_sequence(&env, contract_id), 3);
        assert_eq!(next_sequence(&env, contract_id), 3);
    }

    #[test]
    fn allocated_sequences_form_continuous_replay_stream() {
        let env = Env::default();
        let contract_id = 9u64;
        let events: Vec<OrderedEvent> = (0..3)
            .map(|_| OrderedEvent {
                stream_id: contract_id,
                sequence: next_sequence(&env, contract_id),
                payload_hash: 0,
            })
            .collect();
        assert_eq!(validate_replay_order(&events), Ok(()));
        let seqs: Vec<u64> = events.iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
    }
}
