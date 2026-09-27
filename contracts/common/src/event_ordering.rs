//! Event ordering helpers for versioned Soroban events.
//!
//! This module provides deterministic helpers used by indexers to reason about
//! the ordering and compatibility of versioned event payloads. In particular it
//! exposes a stable schema hash so consumers can detect incompatible payload
//! changes across builds.

use soroban_sdk::{Bytes, Env, Symbol, Vec};

/// Domain separator mixed into every schema hash.
///
/// Kept as a fixed byte string so the hash is stable across builds and cannot
/// collide with hashes produced for other purposes.
const SCHEMA_HASH_DOMAIN: &[u8] = b"soroban-event-schema-v1";

/// Compute a deterministic hash for a versioned event schema.
///
/// The hash is derived only from the event `name`, its `version`, and the
/// ordered list of `fields`. It deliberately avoids any nondeterministic input
/// (timestamps, addresses, map iteration order, etc.) so the value is stable
/// across builds and platforms.
///
/// Because the fields are folded in order, any schema change (renaming a field,
/// adding/removing a field, reordering fields, or bumping the version) alters
/// the resulting hash predictably.
///
/// # Arguments
/// * `env` - The Soroban environment used to allocate the returned `Bytes`.
/// * `name` - The event name (e.g. `Symbol::new(&env, "transfer")`).
/// * `version` - The schema version for the event.
/// * `fields` - The ordered field names that make up the payload schema.
///
/// # Returns
/// A 32-byte `Bytes` value containing the canonical schema hash.
#[must_use]
pub fn schema_hash(env: &Env, name: &Symbol, version: u32, fields: &Vec<Symbol>) -> Bytes {
    let mut preimage = Bytes::new(env);

    // Domain separator first so hashes are namespaced to this helper.
    preimage.extend_from_slice(SCHEMA_HASH_DOMAIN);

    // Length-prefix the event name to avoid ambiguity between concatenated
    // inputs (e.g. "ab" + "c" vs "a" + "bc").
    let name_bytes = name.to_string().to_bytes();
    preimage.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
    preimage.append(&name_bytes);

    // Version is folded in as a fixed-width big-endian integer.
    preimage.extend_from_slice(&version.to_be_bytes());

    // Field count, then each field length-prefixed and in declaration order.
    preimage.extend_from_slice(&(fields.len() as u32).to_be_bytes());
    for field in fields.iter() {
        let field_bytes = field.to_string().to_bytes();
        preimage.extend_from_slice(&(field_bytes.len() as u32).to_be_bytes());
        preimage.append(&field_bytes);
    }

    env.crypto().sha256(&preimage).to_bytes()
}

/// Convenience wrapper that returns the schema hash as a fixed 32-byte array.
///
/// Useful for callers that want to embed the hash directly into an event
/// payload without an extra `Bytes` allocation.
#[must_use]
pub fn schema_hash_array(env: &Env, name: &Symbol, version: u32, fields: &Vec<Symbol>) -> [u8; 32] {
    let hash = schema_hash(env, name, version, fields);
    let mut out = [0u8; 32];
    hash.copy_into_slice(&mut out);
    out
}
