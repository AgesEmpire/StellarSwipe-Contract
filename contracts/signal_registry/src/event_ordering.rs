//! Initial scaffold for deterministic ordering and replay semantics on analytics
//! event streams (#918). Defines an OrderedEvent envelope with a strictly
//! increasing sequence number per stream, and a replay validator that detects
//! out-of-order or duplicate sequences so replays are stable and testable.
//! Follow-up work: wire sequence assignment into the live analytics emission
//! path in analytics_engine.rs.
//!
//! Initial scaffold for deterministic ordering and replay semantics on analytics
//! event streams (#918). Defines an OrderedEvent envelope with a strictly
//! increasing sequence number per stream, and a replay validator that detects
//! out-of-order or duplicate sequences so replays are stable and testable.
//! Follow-up work: wire sequence assignment into the live analytics emission
//! path in analytics_engine.rs.
//!
//! #1107 adds a bounded compaction strategy for replay-protection claim nonces.
//! Consumed claim nonces are retained only within a bounded window; older nonces
//! are evicted (compacted) while newer nonces stay retained. Eviction never
//! reopens a consumed claim: any nonce at or below the compaction watermark is
//! treated as already consumed, so duplicate and reordered claims remain rejected.
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

/// Bounded store of consumed claim nonces for replay protection.
///
/// Nonces are kept in a strictly increasing set. To bound storage growth, the
/// oldest nonces are compacted away once the retained window exceeds
/// `capacity`. Compaction records a `watermark`: every nonce `<= watermark` is
/// considered consumed even though it is no longer stored, so evicted nonces
/// can never be replayed. Nonces above the watermark remain explicitly retained
/// and are still checked for duplicates.
#[derive(Clone)]
pub struct NonceWindow {
    /// Maximum number of nonces retained explicitly before compaction.
    pub capacity: u64,
    /// Highest nonce that has been compacted away. All nonces `<= watermark`
    /// are treated as consumed. `None` means nothing has been compacted yet.
    pub watermark: Option<u64>,
    /// Retained (not yet compacted) consumed nonces, strictly increasing.
    pub retained: Vec<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NonceError {
    /// The nonce was already consumed (either retained or compacted).
    AlreadyConsumed(u64),
    /// The nonce is older than the compaction watermark and cannot be accepted.
    BelowWatermark { watermark: u64, got: u64 },
}

impl NonceWindow {
    /// Creates an empty window with the given retention capacity.
    pub fn new(capacity: u64) -> Self {
        Self { capacity, watermark: None, retained: Vec::new() }
    }

    /// Returns true if `nonce` has already been consumed, whether it is still
    /// retained or has been compacted below the watermark.
    pub fn is_consumed(&self, nonce: u64) -> bool {
        if let Some(watermark) = self.watermark {
            if nonce <= watermark {
                return true;
            }
        }
        self.retained.binary_search(&nonce).is_ok()
    }

    /// Records `nonce` as consumed, rejecting duplicates and nonces that fall
    /// at or below the compaction watermark. After insertion the window is
    /// compacted so at most `capacity` nonces are retained explicitly.
    pub fn consume(&mut self, nonce: u64) -> Result<(), NonceError> {
        if let Some(watermark) = self.watermark {
            if nonce <= watermark {
                return Err(NonceError::BelowWatermark { watermark, got: nonce });
            }
        }
        if self.retained.binary_search(&nonce).is_ok() {
            return Err(NonceError::AlreadyConsumed(nonce));
        }
        let pos = self.retained.binary_search(&nonce).unwrap_or_else(|p| p);
        self.retained.insert(pos, nonce);
        self.compact();
        Ok(())
    }

    /// Compacts the retained set down to `capacity` entries, advancing the
    /// watermark past the evicted (oldest) nonces. Evicted nonces stay consumed
    /// via the watermark, so compaction never reopens a consumed claim.
    pub fn compact(&mut self) {
        if self.capacity == 0 {
            // No explicit retention: everything consumed is folded into the
            // watermark, which still rejects every previously seen nonce.
            if let Some(last) = self.retained.last().copied() {
                self.watermark = Some(match self.watermark {
                    Some(w) => w.max(last),
                    None => last,
                });
                self.retained.clear();
            }
            return;
        }
        while self.retained.len() as u64 > self.capacity {
            let evicted = self.retained.remove(0);
            self.watermark = Some(match self.watermark {
                Some(w) => w.max(evicted),
                None => evicted,
            });
        }
    }
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
    fn consume_rejects_duplicate_before_compaction() {
        let mut window = NonceWindow::new(8);
        assert_eq!(window.consume(1), Ok(()));
        assert_eq!(window.consume(1), Err(NonceError::AlreadyConsumed(1)));
    }

    #[test]
    fn compaction_bounds_retained_nonces() {
        let mut window = NonceWindow::new(3);
        for nonce in 1..=10 {
            assert_eq!(window.consume(nonce), Ok(()));
        }
        assert!(window.retained.len() as u64 <= window.capacity);
        assert_eq!(window.watermark, Some(7));
        assert_eq!(window.retained, vec![8, 9, 10]);
    }

    #[test]
    fn older_nonces_are_consumed_after_compaction() {
        let mut window = NonceWindow::new(2);
        for nonce in 1..=5 {
            assert_eq!(window.consume(nonce), Ok(()));
        }
        // Older (compacted) nonces remain consumed and cannot be replayed.
        assert!(window.is_consumed(1));
        assert_eq!(window.consume(1), Err(NonceError::BelowWatermark { watermark: 3, got: 1 }));
        // Newer (retained) nonces are still explicitly tracked.
        assert!(window.is_consumed(5));
        assert_eq!(window.consume(5), Err(NonceError::AlreadyConsumed(5)));
    }

    #[test]
    fn zero_capacity_still_rejects_all_consumed_nonces() {
        let mut window = NonceWindow::new(0);
        for nonce in 1..=4 {
            assert_eq!(window.consume(nonce), Ok(()));
        }
        assert!(window.retained.is_empty());
        assert_eq!(window.watermark, Some(4));
        for nonce in 1..=4 {
            assert!(window.is_consumed(nonce));
            assert_eq!(window.consume(nonce), Err(NonceError::BelowWatermark { watermark: 4, got: nonce }));
        }
    }

    /// Property test: for any sequence of distinct nonces, replaying any
    /// previously consumed nonce after compaction is always rejected, and the
    /// retained set never exceeds the configured capacity.
    #[test]
    fn property_duplicates_rejected_after_compaction() {
        for capacity in 0..=4u64 {
            for total in 1..=12u64 {
                let mut window = NonceWindow::new(capacity);
                let mut consumed: Vec<u64> = Vec::new();
                for nonce in 1..=total {
                    assert_eq!(window.consume(nonce), Ok(()));
                    consumed.push(nonce);
                    assert!(window.retained.len() as u64 <= capacity);
                }
                // Every previously consumed nonce must still be rejected.
                for &nonce in &consumed {
                    assert!(window.is_consumed(nonce));
                    assert!(window.consume(nonce).is_err());
                }
            }
        }
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

    #[test]
    fn deterministic_sort_produces_stable_order_regardless_of_input_order() {
        let a = deterministic_sort(vec![ev(2, 1), ev(1, 2), ev(1, 1)]);
        let b = deterministic_sort(vec![ev(1, 1), ev(1, 2), ev(2, 1)]);
        let seq_a: Vec<(u64, u64)> = a.iter().map(|e| (e.stream_id, e.sequence)).collect();
        let seq_b: Vec<(u64, u64)> = b.iter().map(|e| (e.stream_id, e.sequence)).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]

    }
}
