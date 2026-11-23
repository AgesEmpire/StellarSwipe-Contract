//! Channel-and-direction scoped replay nonces for the bridge (#1413).
//!
//! Each bridge channel and message direction is an independent nonce stream.
//! A nonce that is valid in one stream cannot be replayed in another, and an
//! equal nonce value in two distinct streams is not a collision.
//!
//! ## Why channel + direction scoping matters
//!
//! Without scoping, a nonce space is global: a message submitted on channel A
//! in the inbound direction occupies the same slot as a message on channel B
//! in the outbound direction. That creates two risks:
//!
//! 1. **Cross-channel replay** – an attacker replays a message from one
//!    channel into another channel that happens to accept the same nonce.
//! 2. **Direction confusion** – an inbound message is accepted by the outbound
//!    path because both paths share a nonce counter.
//!
//! Scoping the nonce key by `(channel_id, direction)` makes each stream
//! independent. A duplicate within a stream is still rejected; equal nonces
//! across distinct streams are permitted and do not conflict.
//!
//! ## Nonce key format
//!
//! ```text
//! nonce_key = sha256(
//!     "bridge-channel-nonce-v1" || 0x00 ||
//!     direction_tag             ||      // 1 byte: 0x01 = Inbound, 0x02 = Outbound
//!     len(channel_id) as u32_be ||
//!     channel_id                ||
//!     nonce as u64_be
//! )
//! ```
//!
//! Domain separation ensures the key space for bridge channel nonces never
//! overlaps with the signal-replay digest space (see `replay_protection`).
//!
//! ## Migration path for existing nonce state
//!
//! Existing global nonces can be migrated by treating them as belonging to a
//! synthetic `("legacy", Inbound)` stream: iterate the existing nonce storage,
//! compute the new scoped key for each entry, and write it into the new store.
//! The old global store can then be cleared. See [`migration`] for helpers.
//!
//! ## What done looks like (#1413)
//!
//! * Nonce keys include the channel and direction dimensions.
//! * Existing nonce state has a defined migration path.
//! * Tests cover duplicates within a stream and equal nonces across distinct
//!   streams.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Domain-separation tag for bridge channel nonce keys.
const DOMAIN_TAG: &[u8] = b"bridge-channel-nonce-v1";

/// Direction of a bridge message.
///
/// `Inbound` messages originate on a foreign chain and are delivered to this
/// contract. `Outbound` messages originate here and are delivered to a foreign
/// chain. The two directions maintain independent nonce sequences so that a
/// message cannot be replayed across directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BridgeDirection {
    /// Message arriving from a foreign chain.
    Inbound,
    /// Message departing to a foreign chain.
    Outbound,
}

impl BridgeDirection {
    /// Returns the stable 1-byte tag encoded into nonce keys.
    pub fn tag(self) -> u8 {
        match self {
            BridgeDirection::Inbound => 0x01,
            BridgeDirection::Outbound => 0x02,
        }
    }

    /// Recovers the direction from its tag byte, if valid.
    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0x01 => Some(BridgeDirection::Inbound),
            0x02 => Some(BridgeDirection::Outbound),
            _ => None,
        }
    }
}

/// A fully-qualified nonce key identifying one message slot within a specific
/// channel and direction stream.
///
/// Keys are compared by their computed digest so that the channel_id and
/// direction are always mixed in and cannot be stripped.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelNonceKey {
    /// Opaque channel identifier (e.g. the peer chain ID or a contract address).
    pub channel_id: Vec<u8>,
    /// Direction of the message stream.
    pub direction: BridgeDirection,
    /// Sequential nonce within the `(channel_id, direction)` stream.
    pub nonce: u64,
}

impl ChannelNonceKey {
    /// Creates a nonce key for `nonce` in the stream identified by
    /// `(channel_id, direction)`.
    pub fn new(
        channel_id: impl Into<Vec<u8>>,
        direction: BridgeDirection,
        nonce: u64,
    ) -> Self {
        Self {
            channel_id: channel_id.into(),
            direction,
            nonce,
        }
    }

    /// Computes the deterministic 32-byte digest used as the storage key.
    ///
    /// The digest mixes the domain tag, direction, channel_id, and nonce so
    /// that equal nonces in different streams produce different digests. This
    /// means the collision property is: two keys collide iff all three of
    /// channel_id, direction, and nonce are identical.
    pub fn digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(DOMAIN_TAG);
        h.update([0u8]); // null byte separator after domain tag
        h.update([self.direction.tag()]);
        h.update((self.channel_id.len() as u32).to_be_bytes());
        h.update(&self.channel_id);
        h.update(self.nonce.to_be_bytes());
        h.finalize().into()
    }
}

/// Error returned when a channel nonce operation is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelNonceError {
    /// The nonce was already consumed in this stream.
    AlreadyConsumed,
    /// The nonce is older than the retention window and has been evicted.
    ///
    /// A claim for an evicted nonce is treated as potentially fresh; callers
    /// that require unbounded protection must persist evicted keys off-chain.
    Evicted,
}

/// Replay protection store scoped to a single `(channel_id, direction)` stream.
///
/// Internally, consumed nonces are stored by their computed digest so the
/// channel and direction dimensions are always mixed into the lookup key.
/// Storage is bounded by `capacity`; once full, the oldest consumed digest is
/// evicted (FIFO) and the nonce slot is considered fresh again.
#[derive(Clone, Debug)]
pub struct ChannelNonceStore {
    /// Set of consumed nonce digests, for O(log n) lookup.
    consumed: BTreeSet<[u8; 32]>,
    /// Insertion-order queue, used to evict the oldest entries.
    order: std::collections::VecDeque<[u8; 32]>,
    /// Maximum number of consumed digests retained before eviction.
    capacity: usize,
}

impl ChannelNonceStore {
    /// Default retention window: 65 536 entries per stream.
    pub const DEFAULT_CAPACITY: usize = 65_536;

    /// Creates a store with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Creates a store retaining at most `capacity` consumed digests.
    ///
    /// A capacity of `0` disables retention: every claim is treated as fresh.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            consumed: BTreeSet::new(),
            order: std::collections::VecDeque::new(),
            capacity,
        }
    }

    /// Attempts to consume `key`.
    ///
    /// Returns `Ok(())` when the nonce is fresh and records it as consumed.
    /// Returns `Err(ChannelNonceError::AlreadyConsumed)` when the nonce was
    /// already consumed and is still within the retention window.
    ///
    /// A nonce whose digest was evicted from the window is treated as fresh
    /// (returns `Ok(())`), consistent with the bounded-window contract.
    pub fn consume(&mut self, key: &ChannelNonceKey) -> Result<(), ChannelNonceError> {
        let digest = key.digest();
        if self.consumed.contains(&digest) {
            return Err(ChannelNonceError::AlreadyConsumed);
        }
        // Evict oldest entry if at capacity.
        if self.capacity > 0 && self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.consumed.remove(&oldest);
            }
        }
        if self.capacity > 0 {
            self.consumed.insert(digest);
            self.order.push_back(digest);
        }
        Ok(())
    }

    /// Returns `true` when `key`'s digest is currently recorded as consumed.
    pub fn is_consumed(&self, key: &ChannelNonceKey) -> bool {
        self.consumed.contains(&key.digest())
    }

    /// Returns the number of consumed digests currently retained.
    pub fn len(&self) -> usize {
        self.consumed.len()
    }

    /// Returns `true` when no consumed digests are retained.
    pub fn is_empty(&self) -> bool {
        self.consumed.is_empty()
    }
}

impl Default for ChannelNonceStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A multi-stream nonce manager that owns one [`ChannelNonceStore`] per
/// `(channel_id, direction)` pair.
///
/// This is the primary entry point for bridge replay protection. The manager
/// routes each consume call to the correct per-stream store, ensuring that
/// nonces from different streams never interfere with each other.
#[derive(Clone, Debug, Default)]
pub struct BridgeNonceManager {
    /// Per-stream stores, keyed by (channel_id_bytes, direction).
    stores: BTreeMap<(Vec<u8>, BridgeDirection), ChannelNonceStore>,
    /// Capacity applied to each newly created stream store.
    stream_capacity: usize,
}

impl BridgeNonceManager {
    /// Creates a manager whose per-stream stores each use the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(ChannelNonceStore::DEFAULT_CAPACITY)
    }

    /// Creates a manager whose per-stream stores each use `capacity`.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            stores: BTreeMap::new(),
            stream_capacity: capacity,
        }
    }

    /// Attempts to consume `key` in its designated stream.
    ///
    /// A new per-stream store is created on first use. Returns
    /// `Err(ChannelNonceError::AlreadyConsumed)` when the nonce was already
    /// consumed in this specific stream; an equal nonce in a different stream
    /// is unaffected.
    pub fn consume(&mut self, key: &ChannelNonceKey) -> Result<(), ChannelNonceError> {
        let stream_key = (key.channel_id.clone(), key.direction);
        let capacity = self.stream_capacity;
        let store = self
            .stores
            .entry(stream_key)
            .or_insert_with(|| ChannelNonceStore::with_capacity(capacity));
        store.consume(key)
    }

    /// Returns `true` when `key`'s nonce is currently recorded as consumed in
    /// its designated stream.
    pub fn is_consumed(&self, key: &ChannelNonceKey) -> bool {
        let stream_key = (key.channel_id.clone(), key.direction);
        self.stores
            .get(&stream_key)
            .map(|s| s.is_consumed(key))
            .unwrap_or(false)
    }

    /// Returns the number of active streams.
    pub fn stream_count(&self) -> usize {
        self.stores.len()
    }
}

// ─── Migration helpers ────────────────────────────────────────────────────────

/// Helpers for migrating existing global nonce state to the
/// channel-and-direction-scoped model (#1413).
pub mod migration {
    use super::*;

    /// The synthetic channel ID assigned to legacy (pre-migration) nonces.
    ///
    /// All existing global nonces are migrated into the
    /// `(LEGACY_CHANNEL_ID, BridgeDirection::Inbound)` stream. This preserves
    /// their replay protection guarantees while making the migration auditable:
    /// the new key space is distinct from all production channel IDs.
    pub const LEGACY_CHANNEL_ID: &[u8] = b"legacy";

    /// A single legacy nonce record, as it would appear in pre-migration
    /// global nonce storage.
    #[derive(Clone, Debug)]
    pub struct LegacyNonce {
        /// The raw sequential nonce value from the old global store.
        pub nonce: u64,
    }

    /// Migrates a slice of legacy global nonces into a [`BridgeNonceManager`].
    ///
    /// Each legacy nonce is inserted into the synthetic
    /// `(LEGACY_CHANNEL_ID, Inbound)` stream. The function is idempotent:
    /// consuming a nonce that is already consumed is silently ignored during
    /// migration (the nonce was already protected).
    ///
    /// Returns the number of nonces successfully migrated (i.e. not already
    /// consumed in the target stream).
    pub fn migrate_legacy_nonces(
        manager: &mut BridgeNonceManager,
        legacy: &[LegacyNonce],
    ) -> usize {
        let mut count = 0usize;
        for record in legacy {
            let key = ChannelNonceKey::new(LEGACY_CHANNEL_ID, BridgeDirection::Inbound, record.nonce);
            if manager.consume(&key).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Returns the scoped nonce key that a migrated legacy nonce maps to.
    ///
    /// SDK and indexer authors can use this to verify that a given legacy
    /// nonce was correctly migrated.
    pub fn legacy_nonce_key(nonce: u64) -> ChannelNonceKey {
        ChannelNonceKey::new(LEGACY_CHANNEL_ID, BridgeDirection::Inbound, nonce)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::migration::*;

    fn ch(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    // ── BridgeDirection ──────────────────────────────────────────────────────

    #[test]
    fn direction_tags_are_distinct() {
        assert_ne!(BridgeDirection::Inbound.tag(), BridgeDirection::Outbound.tag());
    }

    #[test]
    fn direction_round_trips_through_tag() {
        for dir in [BridgeDirection::Inbound, BridgeDirection::Outbound] {
            assert_eq!(BridgeDirection::from_tag(dir.tag()), Some(dir));
        }
    }

    #[test]
    fn unknown_tag_returns_none() {
        assert_eq!(BridgeDirection::from_tag(0x00), None);
        assert_eq!(BridgeDirection::from_tag(0xFF), None);
    }

    // ── ChannelNonceKey digest uniqueness ────────────────────────────────────

    #[test]
    fn equal_nonce_in_different_channels_produces_different_digests() {
        let k1 = ChannelNonceKey::new(ch("channel-a"), BridgeDirection::Inbound, 42);
        let k2 = ChannelNonceKey::new(ch("channel-b"), BridgeDirection::Inbound, 42);
        assert_ne!(k1.digest(), k2.digest());
    }

    #[test]
    fn equal_nonce_in_different_directions_produces_different_digests() {
        let k1 = ChannelNonceKey::new(ch("channel-a"), BridgeDirection::Inbound, 42);
        let k2 = ChannelNonceKey::new(ch("channel-a"), BridgeDirection::Outbound, 42);
        assert_ne!(k1.digest(), k2.digest());
    }

    #[test]
    fn identical_key_produces_same_digest() {
        let k1 = ChannelNonceKey::new(ch("channel-a"), BridgeDirection::Inbound, 42);
        let k2 = ChannelNonceKey::new(ch("channel-a"), BridgeDirection::Inbound, 42);
        assert_eq!(k1.digest(), k2.digest());
    }

    // ── ChannelNonceStore ────────────────────────────────────────────────────

    #[test]
    fn fresh_nonce_is_accepted() {
        let mut store = ChannelNonceStore::new();
        let key = ChannelNonceKey::new(ch("ch"), BridgeDirection::Inbound, 1);
        assert_eq!(store.consume(&key), Ok(()));
    }

    #[test]
    fn duplicate_within_stream_is_rejected() {
        let mut store = ChannelNonceStore::new();
        let key = ChannelNonceKey::new(ch("ch"), BridgeDirection::Inbound, 1);
        store.consume(&key).unwrap();
        assert_eq!(store.consume(&key), Err(ChannelNonceError::AlreadyConsumed));
    }

    #[test]
    fn eviction_allows_old_nonce_to_be_reused() {
        // Capacity 2: consuming 3 entries evicts the first.
        let mut store = ChannelNonceStore::with_capacity(2);
        let k0 = ChannelNonceKey::new(ch("ch"), BridgeDirection::Inbound, 0);
        let k1 = ChannelNonceKey::new(ch("ch"), BridgeDirection::Inbound, 1);
        let k2 = ChannelNonceKey::new(ch("ch"), BridgeDirection::Inbound, 2);
        store.consume(&k0).unwrap();
        store.consume(&k1).unwrap();
        // This evicts k0.
        store.consume(&k2).unwrap();
        // k0 is gone, k0 is now fresh.
        assert!(!store.is_consumed(&k0));
        // k1 and k2 are still retained.
        assert!(store.is_consumed(&k1));
        assert!(store.is_consumed(&k2));
    }

    // ── BridgeNonceManager ───────────────────────────────────────────────────

    #[test]
    fn equal_nonce_in_distinct_streams_does_not_conflict() {
        let mut mgr = BridgeNonceManager::new();
        let k1 = ChannelNonceKey::new(ch("chain-a"), BridgeDirection::Inbound, 100);
        let k2 = ChannelNonceKey::new(ch("chain-b"), BridgeDirection::Inbound, 100);
        // Both should succeed: different channels.
        assert_eq!(mgr.consume(&k1), Ok(()));
        assert_eq!(mgr.consume(&k2), Ok(()));
        // And re-use within each stream is blocked.
        assert_eq!(mgr.consume(&k1), Err(ChannelNonceError::AlreadyConsumed));
        assert_eq!(mgr.consume(&k2), Err(ChannelNonceError::AlreadyConsumed));
    }

    #[test]
    fn equal_nonce_in_inbound_and_outbound_does_not_conflict() {
        let mut mgr = BridgeNonceManager::new();
        let k_in = ChannelNonceKey::new(ch("chain-a"), BridgeDirection::Inbound, 1);
        let k_out = ChannelNonceKey::new(ch("chain-a"), BridgeDirection::Outbound, 1);
        assert_eq!(mgr.consume(&k_in), Ok(()));
        // Outbound with same channel+nonce is a distinct stream: not a conflict.
        assert_eq!(mgr.consume(&k_out), Ok(()));
    }

    #[test]
    fn duplicate_within_same_stream_is_blocked_by_manager() {
        let mut mgr = BridgeNonceManager::new();
        let key = ChannelNonceKey::new(ch("chain-a"), BridgeDirection::Outbound, 7);
        mgr.consume(&key).unwrap();
        assert_eq!(mgr.consume(&key), Err(ChannelNonceError::AlreadyConsumed));
    }

    #[test]
    fn manager_creates_independent_stream_per_channel() {
        let mut mgr = BridgeNonceManager::new();
        for i in 0..5u64 {
            let key = ChannelNonceKey::new(format!("ch-{i}").into_bytes(), BridgeDirection::Inbound, i);
            mgr.consume(&key).unwrap();
        }
        assert_eq!(mgr.stream_count(), 5);
    }

    // ── Migration ────────────────────────────────────────────────────────────

    #[test]
    fn legacy_nonces_migrate_into_legacy_channel_inbound_stream() {
        let legacy = vec![
            LegacyNonce { nonce: 0 },
            LegacyNonce { nonce: 1 },
            LegacyNonce { nonce: 2 },
        ];
        let mut mgr = BridgeNonceManager::new();
        let migrated = migrate_legacy_nonces(&mut mgr, &legacy);
        assert_eq!(migrated, 3);

        // All three are now consumed in the legacy stream.
        for n in 0u64..3 {
            let key = legacy_nonce_key(n);
            assert!(mgr.is_consumed(&key));
        }
    }

    #[test]
    fn migrated_legacy_nonces_are_blocked_from_replay() {
        let mut mgr = BridgeNonceManager::new();
        migrate_legacy_nonces(&mut mgr, &[LegacyNonce { nonce: 42 }]);
        // Attempting to consume again should fail.
        assert_eq!(
            mgr.consume(&legacy_nonce_key(42)),
            Err(ChannelNonceError::AlreadyConsumed)
        );
    }

    #[test]
    fn legacy_nonce_does_not_block_production_channel_with_same_nonce() {
        let mut mgr = BridgeNonceManager::new();
        migrate_legacy_nonces(&mut mgr, &[LegacyNonce { nonce: 42 }]);
        // A production channel with the same nonce value is unaffected.
        let prod_key = ChannelNonceKey::new(ch("production-chain"), BridgeDirection::Inbound, 42);
        assert_eq!(mgr.consume(&prod_key), Ok(()));
    }
}
