//! Source provenance metadata for accepted oracle price rounds (Issue #1410).
//!
//! Every accepted price observation is backed by one or more contributing
//! sources. This module persists that set — source addresses and their
//! per-round sequence numbers — atomically alongside the price round so
//! downstream consumers can audit exactly who contributed to each result.
//!
//! # Guarantees
//! - Provenance is written in the same storage transaction as the accepted
//!   price round, so it is never out of step with it.
//! - Rejected observations (bad deviation, insufficient quorum, stale round)
//!   do not alter stored provenance; the previous round's provenance remains
//!   intact until a new round is successfully accepted.
//! - The source set is bounded by [`MAX_PROVENANCE_SOURCES`] to keep storage
//!   costs predictable.

use soroban_sdk::{contracttype, Address, Env, Vec};
use stellar_swipe_common::AssetPair;

// ── Bounds ────────────────────────────────────────────────────────────────────

/// Maximum number of contributing sources recorded per accepted round.
/// Matches the practical quorum upper bound; the caller trims to this limit
/// before passing the slice to [`write_provenance`].
pub const MAX_PROVENANCE_SOURCES: u32 = 16;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Identity of a single source that contributed to an accepted price round.
///
/// `sequence` is the feed-local or ledger-level sequence at which the source's
/// observation was recorded. Together with `source` it uniquely identifies
/// the observation (see `observation::admit`) and lets auditors verify that
/// the same `(source, sequence)` pair cannot be counted more than once.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceEntry {
    /// Oracle address that submitted the contributing observation.
    pub source: Address,
    /// Feed-local ledger sequence of the contributing observation.
    pub sequence: u32,
}

/// Provenance metadata stored alongside an accepted price round.
///
/// Persisted atomically with the round write so it is always consistent with
/// the price visible through `get_price_round`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundProvenance {
    /// Feed-local round identifier this provenance belongs to.
    pub round_id: u64,
    /// Ordered list of contributing sources (bounded by [`MAX_PROVENANCE_SOURCES`]).
    pub sources: Vec<ProvenanceEntry>,
    /// Ledger timestamp at which the round was accepted.
    pub accepted_at: u64,
}

// ── Storage key ───────────────────────────���───────────────────────────────────

#[contracttype]
#[derive(Clone)]
enum ProvenanceKey {
    /// Latest accepted round provenance for an asset pair.
    RoundProvenance(AssetPair),
}

// ── Write ─────────────────────────────────────────────────────────────────────

/// Persist provenance for `round_id` of `pair`.
///
/// Must be called in the same code path that accepts and stores the price
/// round so the write is effectively atomic. Callers build `sources` from
/// the contributing `PriceData` set after quorum is verified.
///
/// # Panics
/// Does not panic; silently trims `sources` to [`MAX_PROVENANCE_SOURCES`]
/// if the caller supplies more (which should not happen under normal quorum
/// bounds).
pub fn write_provenance(
    env: &Env,
    pair: &AssetPair,
    round_id: u64,
    sources: Vec<ProvenanceEntry>,
) {
    let trimmed = if sources.len() > MAX_PROVENANCE_SOURCES {
        let mut v = Vec::new(env);
        for i in 0..MAX_PROVENANCE_SOURCES {
            v.push_back(sources.get_unchecked(i));
        }
        v
    } else {
        sources
    };

    let record = RoundProvenance {
        round_id,
        sources: trimmed,
        accepted_at: env.ledger().timestamp(),
    };

    env.storage()
        .persistent()
        .set(&ProvenanceKey::RoundProvenance(pair.clone()), &record);
}

// ── Read ──────────────────────────────────────────────────────────────────────

/// Return the stored provenance for `pair`'s latest accepted round,
/// or `None` when no round has been accepted yet.
pub fn read_provenance(env: &Env, pair: &AssetPair) -> Option<RoundProvenance> {
    env.storage()
        .persistent()
        .get(&ProvenanceKey::RoundProvenance(pair.clone()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        vec, Env,
    };
    use stellar_swipe_common::{Asset, AssetPair};

    fn pair(env: &Env) -> AssetPair {
        AssetPair {
            base: Asset {
                code: soroban_sdk::String::from_str(env, "XLM"),
                issuer: None,
            },
            quote: Asset {
                code: soroban_sdk::String::from_str(env, "USDC"),
                issuer: None,
            },
        }
    }

    fn entry(env: &Env, sequence: u32) -> ProvenanceEntry {
        ProvenanceEntry {
            source: Address::generate(env),
            sequence,
        }
    }

    // ── Accepted observations write provenance ────────────────────────────────

    #[test]
    fn write_and_read_provenance_round_trip() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let p = pair(&env);

        let sources = vec![&env, entry(&env, 1), entry(&env, 2), entry(&env, 3)];
        write_provenance(&env, &p, 7, sources.clone());

        let prov = read_provenance(&env, &p).expect("provenance must be stored");
        assert_eq!(prov.round_id, 7);
        assert_eq!(prov.accepted_at, 1_000);
        assert_eq!(prov.sources.len(), 3);
        for i in 0..3u32 {
            assert_eq!(
                prov.sources.get_unchecked(i).sequence,
                sources.get_unchecked(i).sequence
            );
        }
    }

    // ── Provenance is overwritten atomically on the next accepted round ───────

    #[test]
    fn later_round_overwrites_provenance() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let p = pair(&env);

        let s1 = vec![&env, entry(&env, 1)];
        write_provenance(&env, &p, 1, s1);

        env.ledger().set_timestamp(2_000);
        let s2 = vec![&env, entry(&env, 2), entry(&env, 3)];
        write_provenance(&env, &p, 2, s2);

        let prov = read_provenance(&env, &p).unwrap();
        assert_eq!(prov.round_id, 2);
        assert_eq!(prov.accepted_at, 2_000);
        assert_eq!(prov.sources.len(), 2);
    }

    // ── Rejected observations must not alter stored provenance ────────────────
    //
    // The caller is responsible for calling write_provenance only after the
    // round has been accepted. This test verifies that a failed acceptance
    // (simulated by simply not calling write_provenance) leaves prior
    // provenance intact.

    #[test]
    fn rejected_observation_does_not_alter_provenance() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let p = pair(&env);

        let sources = vec![&env, entry(&env, 5), entry(&env, 6)];
        write_provenance(&env, &p, 10, sources);

        // Simulate a rejected observation: write_provenance is NOT called.
        // The existing provenance from round 10 must be unchanged.

        let prov = read_provenance(&env, &p).unwrap();
        assert_eq!(prov.round_id, 10);
        assert_eq!(prov.sources.len(), 2);
    }

    // ── No provenance before any round is accepted ────────────────────────────

    #[test]
    fn no_provenance_before_first_round() {
        let env = Env::default();
        let p = pair(&env);
        assert!(read_provenance(&env, &p).is_none());
    }

    // ── Provenance is trimmed to MAX_PROVENANCE_SOURCES ───────────────────────

    #[test]
    fn oversized_source_set_is_trimmed() {
        let env = Env::default();
        env.ledger().set_timestamp(500);
        let p = pair(&env);

        let mut big = Vec::new(&env);
        for i in 0..(MAX_PROVENANCE_SOURCES + 5) {
            big.push_back(entry(&env, i));
        }
        write_provenance(&env, &p, 1, big);

        let prov = read_provenance(&env, &p).unwrap();
        assert_eq!(prov.sources.len(), MAX_PROVENANCE_SOURCES);
    }

    // ── Sources and sequence numbers are faithfully preserved ─────────────────

    #[test]
    fn source_identities_and_sequences_preserved() {
        let env = Env::default();
        env.ledger().set_timestamp(999);
        let p = pair(&env);

        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let sources = vec![
            &env,
            ProvenanceEntry {
                source: a.clone(),
                sequence: 42,
            },
            ProvenanceEntry {
                source: b.clone(),
                sequence: 43,
            },
        ];
        write_provenance(&env, &p, 5, sources);

        let prov = read_provenance(&env, &p).unwrap();
        assert_eq!(prov.sources.get_unchecked(0).source, a);
        assert_eq!(prov.sources.get_unchecked(0).sequence, 42);
        assert_eq!(prov.sources.get_unchecked(1).source, b);
        assert_eq!(prov.sources.get_unchecked(1).sequence, 43);
    }

    // ── Distinct pairs maintain independent provenance ─────────────────────────

    #[test]
    fn distinct_pairs_have_independent_provenance() {
        let env = Env::default();
        env.ledger().set_timestamp(100);

        let p1 = AssetPair {
            base: Asset {
                code: soroban_sdk::String::from_str(&env, "XLM"),
                issuer: None,
            },
            quote: Asset {
                code: soroban_sdk::String::from_str(&env, "USDC"),
                issuer: None,
            },
        };
        let p2 = AssetPair {
            base: Asset {
                code: soroban_sdk::String::from_str(&env, "BTC"),
                issuer: None,
            },
            quote: Asset {
                code: soroban_sdk::String::from_str(&env, "USDC"),
                issuer: None,
            },
        };

        let s1 = vec![&env, entry(&env, 1)];
        let s2 = vec![&env, entry(&env, 2), entry(&env, 3)];
        write_provenance(&env, &p1, 1, s1);
        write_provenance(&env, &p2, 99, s2);

        assert_eq!(read_provenance(&env, &p1).unwrap().round_id, 1);
        assert_eq!(read_provenance(&env, &p2).unwrap().round_id, 99);
        assert_eq!(read_provenance(&env, &p2).unwrap().sources.len(), 2);
    }
}
