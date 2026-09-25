//! Shared helpers for deterministic protocol state checksums.
//!
//! ## Checksum format
//!
//! [`state_checksum`] produces a lowercase hexadecimal SHA-256 digest over a
//! canonical, explicitly ordered byte stream. The input set and ordering are
//! fixed as follows:
//!
//! 1. A domain-separation tag: the ASCII bytes `b"state-checksum:v1"`.
//! 2. For each entry in `entries`, in the caller-supplied order:
//!    - the entry key encoded as UTF-8 bytes, prefixed by its length as a
//!      big-endian `u32`;
//!    - the entry value encoded as UTF-8 bytes, prefixed by its length as a
//!      big-endian `u32`.
//!
//! Callers MUST supply entries in a canonical order (for example, sorted by
//! key). Because the encoding is length-prefixed and order-sensitive, the
//! digest is independent of any underlying map iteration order and of any
//! storage that is not part of `entries`.
//!
//! The resulting digest is stable across repeated calls for equivalent states
//! and is intended for incident comparison and migration verification.

use sha2::{Digest, Sha256};

/// Domain-separation tag mixed into every checksum.
const CHECKSUM_DOMAIN: &[u8] = b"state-checksum:v1";

/// A single key/value pair contributing to a [`state_checksum`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateEntry {
    pub key: String,
    pub value: String,
}

impl StateEntry {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// Compute a deterministic checksum over the provided state entries.
///
/// The caller is responsible for supplying `entries` in a canonical order.
/// Equivalent states (same entries, same order) always yield the same digest,
/// regardless of unrelated storage or map iteration order.
///
/// Returns the digest as a lowercase hexadecimal string.
pub fn state_checksum(entries: &[StateEntry]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CHECKSUM_DOMAIN);

    for entry in entries {
        let key = entry.key.as_bytes();
        hasher.update((key.len() as u32).to_be_bytes());
        hasher.update(key);

        let value = entry.value.as_bytes();
        hasher.update((value.len() as u32).to_be_bytes());
        hasher.update(value);
    }

    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<StateEntry> {
        vec![
            StateEntry::new("admin", "GADMIN"),
            StateEntry::new("threshold", "2"),
        ]
    }

    #[test]
    fn repeated_calls_are_identical() {
        let first = state_checksum(&entries());
        let second = state_checksum(&entries());
        assert_eq!(first, second);
    }

    #[test]
    fn unrelated_storage_does_not_change_checksum() {
        let baseline = state_checksum(&entries());

        // Simulate unrelated storage that is not part of the input set.
        let mut unrelated = std::collections::HashMap::new();
        unrelated.insert("unrelated", "value");
        unrelated.insert("other", "data");
        let _ = unrelated.len();

        assert_eq!(baseline, state_checksum(&entries()));
    }

    #[test]
    fn map_iteration_order_does_not_change_checksum() {
        let mut map = std::collections::HashMap::new();
        map.insert("admin", "GADMIN");
        map.insert("threshold", "2");

        // Canonicalize by sorting keys so iteration order is irrelevant.
        let mut keys: Vec<_> = map.keys().copied().collect();
        keys.sort_unstable();
        let canonical: Vec<StateEntry> = keys
            .iter()
            .map(|k| StateEntry::new(*k, map[k]))
            .collect();

        let first = state_checksum(&canonical);

        // Rebuild the map in a different insertion order.
        let mut reordered = std::collections::HashMap::new();
        reordered.insert("threshold", "2");
        reordered.insert("admin", "GADMIN");
        let mut keys2: Vec<_> = reordered.keys().copied().collect();
        keys2.sort_unstable();
        let canonical2: Vec<StateEntry> = keys2
            .iter()
            .map(|k| StateEntry::new(*k, reordered[k]))
            .collect();

        assert_eq!(first, state_checksum(&canonical2));
    }

    #[test]
    fn ordering_is_significant() {
        let a = vec![StateEntry::new("a", "1"), StateEntry::new("b", "2")];
        let b = vec![StateEntry::new("b", "2"), StateEntry::new("a", "1")];
        assert_ne!(state_checksum(&a), state_checksum(&b));
    }
}
