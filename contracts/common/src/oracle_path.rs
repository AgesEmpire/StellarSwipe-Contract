//! Deterministic multi-hop oracle path selection (#1411).
//!
//! When multiple route candidates exist between a source asset and a target
//! asset, the selection must be deterministic regardless of the order in which
//! candidates were discovered or stored. This module defines path ranking with
//! documented stable tie-breaks and enforces explicit hop and work limits so
//! that evaluation is always bounded.
//!
//! ## Path ranking
//!
//! Candidates are ranked in order of:
//!
//! 1. **Fewest hops** (ascending) — shorter paths incur less slippage and
//!    execution cost.
//! 2. **Best output quote** (descending) — among equally short paths, the one
//!    that produces the most output tokens is preferred.
//! 3. **Lexicographic hop sequence** (ascending) — when hops and quotes are
//!    equal, the path whose string of pool IDs comes first in byte order wins.
//!    This is the stable tie-break: it is deterministic regardless of the input
//!    ordering.
//!
//! The tie-break on lexicographic hop sequence means that two callers who
//! present the same set of candidates in different orders always select the
//! same route.
//!
//! ## Hop and work limits
//!
//! [`MAX_ORACLE_HOPS`] caps the number of hops in any single candidate. Any
//! candidate with more hops is silently skipped during evaluation.
//!
//! [`MAX_ORACLE_CANDIDATES`] caps the number of candidates evaluated per call.
//! Candidates beyond this limit are silently skipped. This prevents unbounded
//! work when a caller supplies a large candidate set.
//!
//! ## What done looks like (#1411)
//!
//! * Path ranking has documented stable tie-breaks.
//! * Candidate evaluation is bounded by an explicit hop and work limit.
//! * Permutation tests produce the same route and quote.

/// Maximum number of hops in a single oracle route candidate.
///
/// A candidate with more hops than this is skipped during evaluation.
pub const MAX_ORACLE_HOPS: usize = 5;

/// Maximum number of candidates evaluated per [`select_oracle_path`] call.
///
/// Candidates beyond this index are silently skipped. Callers that supply
/// more candidates must pre-filter them to this limit.
pub const MAX_ORACLE_CANDIDATES: usize = 128;

/// A single hop in an oracle route, identified by an opaque pool/feed ID.
///
/// The ID is a 32-byte opaque identifier (e.g. a pool address hash or a
/// price-feed key). Two hops with the same ID are considered identical.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OracleHop {
    /// Opaque 32-byte pool or feed identifier.
    pub pool_id: [u8; 32],
}

impl OracleHop {
    /// Creates a hop from a 32-byte pool identifier.
    pub fn new(pool_id: [u8; 32]) -> Self {
        Self { pool_id }
    }
}

/// A candidate oracle route from a source asset to a target asset.
///
/// A route is an ordered sequence of [`OracleHop`]s. The quote is the
/// expected output amount in the target asset's base unit (e.g. the smallest
/// divisible unit). A higher quote means the path delivers more output for
/// the same input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleRoute {
    /// Ordered sequence of hops from source to target.
    pub hops: Vec<OracleHop>,
    /// Expected output amount for a canonical unit of the source asset.
    ///
    /// A value of `0` means the route produces no output (effectively
    /// unusable), but it is not a structural error. Routes with quote 0 are
    /// ranked last among same-hop-count paths.
    pub quote: u128,
}

impl OracleRoute {
    /// Creates a route with explicit hops and a pre-computed quote.
    pub fn new(hops: Vec<OracleHop>, quote: u128) -> Self {
        Self { hops, quote }
    }

    /// Returns the number of hops in this route.
    pub fn hop_count(&self) -> usize {
        self.hops.len()
    }

    /// Computes the stable sort key used to rank this route among candidates.
    ///
    /// The key is a tuple `(hop_count, inverted_quote, hop_sequence)` where:
    /// - `hop_count` sorts ascending (fewer hops first).
    /// - `inverted_quote` is `u128::MAX - quote`, so descending quote order
    ///   maps to ascending key order (enabling a single `sort` direction).
    /// - `hop_sequence` is the concatenated `pool_id` bytes, used as the
    ///   lexicographic tie-break.
    ///
    /// Because tuples are compared lexicographically, sorting by this key in
    /// ascending order implements the full three-level ranking.
    pub fn sort_key(&self) -> (usize, u128, Vec<u8>) {
        let hop_count = self.hop_count();
        let inverted_quote = u128::MAX - self.quote;
        let hop_sequence: Vec<u8> = self.hops.iter().flat_map(|h| h.pool_id).collect();
        (hop_count, inverted_quote, hop_sequence)
    }
}

/// Errors returned by [`select_oracle_path`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OraclePathError {
    /// No candidate passed the hop-count and work-limit filters.
    NoCandidates,
    /// Every candidate exceeds the [`MAX_ORACLE_HOPS`] limit.
    AllCandidatesExceedHopLimit,
}

/// Selects the best oracle route from `candidates`.
///
/// Candidates are filtered to those with at most [`MAX_ORACLE_HOPS`] hops and
/// the first [`MAX_ORACLE_CANDIDATES`] entries. The surviving candidates are
/// ranked by the three-level sort key (see [`OracleRoute::sort_key`]) and the
/// top-ranked candidate is returned.
///
/// The selection is deterministic: for any fixed set of candidates, the same
/// route is always returned regardless of the order in which the candidates
/// were supplied to this function.
///
/// Returns:
/// - `Ok(&OracleRoute)` — the best route.
/// - `Err(OraclePathError::NoCandidates)` — the slice is empty after applying
///   the work limit.
/// - `Err(OraclePathError::AllCandidatesExceedHopLimit)` — all candidates were
///   filtered out by the hop limit.
pub fn select_oracle_path(candidates: &[OracleRoute]) -> Result<&OracleRoute, OraclePathError> {
    if candidates.is_empty() {
        return Err(OraclePathError::NoCandidates);
    }

    // Apply work limit: consider at most MAX_ORACLE_CANDIDATES entries.
    let work_set = &candidates[..candidates.len().min(MAX_ORACLE_CANDIDATES)];

    // Filter out routes that exceed the hop limit.
    let eligible: Vec<&OracleRoute> = work_set
        .iter()
        .filter(|r| r.hop_count() <= MAX_ORACLE_HOPS && r.hop_count() > 0)
        .collect();

    if eligible.is_empty() {
        return Err(OraclePathError::AllCandidatesExceedHopLimit);
    }

    // Select the route with the smallest sort key (see OracleRoute::sort_key).
    // Using min_by_key gives us a deterministic single-pass selection without
    // requiring allocation of sorted indices.
    let best = eligible
        .into_iter()
        .min_by_key(|r| r.sort_key())
        .expect("eligible is non-empty");

    Ok(best)
}

/// Selects the best oracle path from `candidates`, consuming ownership and
/// returning the winning route by value.
///
/// Semantics are identical to [`select_oracle_path`], but ownership is
/// transferred so the caller does not need to manage lifetimes.
pub fn select_oracle_path_owned(
    mut candidates: Vec<OracleRoute>,
) -> Result<OracleRoute, OraclePathError> {
    if candidates.is_empty() {
        return Err(OraclePathError::NoCandidates);
    }

    // Apply work limit.
    candidates.truncate(MAX_ORACLE_CANDIDATES);

    // Filter out routes that exceed the hop limit or are empty.
    candidates.retain(|r| r.hop_count() > 0 && r.hop_count() <= MAX_ORACLE_HOPS);

    if candidates.is_empty() {
        return Err(OraclePathError::AllCandidatesExceedHopLimit);
    }

    // Sort ascending by sort key to place the best route at index 0.
    candidates.sort_by_key(|r| r.sort_key());

    Ok(candidates.remove(0))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(n: u8) -> OracleHop {
        OracleHop::new([n; 32])
    }

    fn route(hops: &[OracleHop], quote: u128) -> OracleRoute {
        OracleRoute::new(hops.to_vec(), quote)
    }

    // ── Empty / invalid input ─────────────────────────────────────────────────

    #[test]
    fn empty_candidates_returns_no_candidates_error() {
        let result = select_oracle_path(&[]);
        assert_eq!(result, Err(OraclePathError::NoCandidates));
    }

    #[test]
    fn all_candidates_exceeding_hop_limit_returns_error() {
        let r = route(&vec![pool(0); MAX_ORACLE_HOPS + 1], 100);
        let result = select_oracle_path(&[r]);
        assert_eq!(result, Err(OraclePathError::AllCandidatesExceedHopLimit));
    }

    // ── Single candidate ──────────────────────────────────────────────────────

    #[test]
    fn single_valid_candidate_is_selected() {
        let r = route(&[pool(1), pool(2)], 500);
        let result = select_oracle_path(std::slice::from_ref(&r)).unwrap();
        assert_eq!(result, &r);
    }

    // ── Hop-count ranking ─────────────────────────────────────────────────────

    #[test]
    fn fewer_hops_wins_over_higher_quote() {
        // 1-hop route with quote 10 should beat 2-hop route with quote 1000.
        let short = route(&[pool(1)], 10);
        let long = route(&[pool(1), pool(2)], 1_000);
        let candidates = [short.clone(), long.clone()];
        let best = select_oracle_path(&candidates).unwrap();
        assert_eq!(best, &short);
    }

    #[test]
    fn among_same_hop_count_higher_quote_wins() {
        let lower = route(&[pool(1), pool(2)], 100);
        let higher = route(&[pool(3), pool(4)], 200);
        let candidates = [lower.clone(), higher.clone()];
        let best = select_oracle_path(&candidates).unwrap();
        assert_eq!(best, &higher);
    }

    // ── Tie-break: lexicographic hop sequence ─────────────────────────────────

    #[test]
    fn equal_hops_and_quote_uses_lexicographic_tie_break() {
        // Two 1-hop routes with the same quote but different pool IDs.
        // pool(0x01) < pool(0xFF) lexicographically, so pool(0x01) wins.
        let a = route(&[pool(0x01)], 100);
        let b = route(&[pool(0xFF)], 100);
        let candidates = [b.clone(), a.clone()]; // supply b first
        let best = select_oracle_path(&candidates).unwrap();
        assert_eq!(best, &a); // a wins despite being listed second
    }

    #[test]
    fn tie_break_on_second_hop_when_first_is_equal() {
        let a = route(&[pool(0x01), pool(0x01)], 100);
        let b = route(&[pool(0x01), pool(0xFF)], 100);
        // First hop is equal; second hop breaks the tie.
        let candidates = [b.clone(), a.clone()];
        let best = select_oracle_path(&candidates).unwrap();
        assert_eq!(best, &a);
    }

    // ── Permutation invariance ────────────────────────────────────────────────

    #[test]
    fn all_permutations_of_three_candidates_select_same_route() {
        let r1 = route(&[pool(0x01)], 300);           // 1 hop, best quote
        let r2 = route(&[pool(0x02), pool(0x03)], 500); // 2 hops
        let r3 = route(&[pool(0x04)], 200);           // 1 hop, lower quote

        // r1 wins: 1 hop, quote 300 > 200.
        let permutations: &[&[OracleRoute]] = &[
            &[r1.clone(), r2.clone(), r3.clone()],
            &[r1.clone(), r3.clone(), r2.clone()],
            &[r2.clone(), r1.clone(), r3.clone()],
            &[r2.clone(), r3.clone(), r1.clone()],
            &[r3.clone(), r1.clone(), r2.clone()],
            &[r3.clone(), r2.clone(), r1.clone()],
        ];
        for perm in permutations {
            let best = select_oracle_path(perm).unwrap();
            assert_eq!(best, &r1, "permutation did not select r1: {perm:?}");
        }
    }

    #[test]
    fn permutation_invariance_with_tie_break() {
        // All candidates have 2 hops and quote 100; lex tie-break selects the
        // candidate whose pool sequence comes first.
        let winner = route(&[pool(0x01), pool(0x02)], 100); // lex smallest
        let other_a = route(&[pool(0x05), pool(0x06)], 100);
        let other_b = route(&[pool(0x0A), pool(0x0B)], 100);

        let permutations: &[&[OracleRoute]] = &[
            &[winner.clone(), other_a.clone(), other_b.clone()],
            &[other_a.clone(), winner.clone(), other_b.clone()],
            &[other_b.clone(), other_a.clone(), winner.clone()],
            &[other_a.clone(), other_b.clone(), winner.clone()],
            &[other_b.clone(), winner.clone(), other_a.clone()],
        ];
        for perm in permutations {
            let best = select_oracle_path(perm).unwrap();
            assert_eq!(best, &winner, "permutation did not select winner: {perm:?}");
        }
    }

    // ── Work limit ────────────────────────────────────────────────────────────

    #[test]
    fn candidates_beyond_work_limit_are_ignored() {
        // Fill MAX_ORACLE_CANDIDATES slots with quote-100 routes, then add one
        // more with quote 999 (which should be ignored because it exceeds the
        // work limit).
        let ordinary = route(&[pool(0x01)], 100);
        let beyond_limit = route(&[pool(0x02)], 999);

        let mut candidates: Vec<OracleRoute> = vec![ordinary.clone(); MAX_ORACLE_CANDIDATES];
        candidates.push(beyond_limit.clone());

        let best = select_oracle_path(&candidates).unwrap();
        // The best among the first MAX_ORACLE_CANDIDATES ordinary routes wins;
        // beyond_limit is not considered.
        assert_ne!(best, &beyond_limit);
    }

    // ── Hop limit filtering ───────────────────────────────────────────────────

    #[test]
    fn candidates_exceeding_hop_limit_are_filtered_out() {
        let too_long = route(&vec![pool(0x01); MAX_ORACLE_HOPS + 1], 9_999);
        let valid = route(&[pool(0x02)], 1);
        let candidates = [too_long.clone(), valid.clone()];
        let best = select_oracle_path(&candidates).unwrap();
        // too_long is skipped; valid is returned despite lower quote.
        assert_eq!(best, &valid);
    }

    #[test]
    fn route_exactly_at_max_hops_is_accepted() {
        let at_limit = route(&vec![pool(0x01); MAX_ORACLE_HOPS], 100);
        let result = select_oracle_path(std::slice::from_ref(&at_limit));
        assert!(result.is_ok());
    }

    // ── select_oracle_path_owned ──────────────────────────────────────────────

    #[test]
    fn owned_selection_produces_same_result_as_ref_selection() {
        let r1 = route(&[pool(0x01)], 500);
        let r2 = route(&[pool(0x02), pool(0x03)], 1_000);
        let r3 = route(&[pool(0x04)], 300);

        let candidates_ref = [r1.clone(), r2.clone(), r3.clone()];
        let best_ref = select_oracle_path(&candidates_ref).unwrap().clone();

        let candidates_owned = vec![r1, r2, r3];
        let best_owned = select_oracle_path_owned(candidates_owned).unwrap();

        assert_eq!(best_ref, best_owned);
    }

    #[test]
    fn owned_selection_permutation_invariance() {
        let winner = route(&[pool(0x01)], 500);
        let loser_a = route(&[pool(0x02)], 400);
        let loser_b = route(&[pool(0x03)], 300);

        let permutations: Vec<Vec<OracleRoute>> = vec![
            vec![winner.clone(), loser_a.clone(), loser_b.clone()],
            vec![loser_b.clone(), winner.clone(), loser_a.clone()],
            vec![loser_a.clone(), loser_b.clone(), winner.clone()],
        ];
        for perm in permutations {
            let best = select_oracle_path_owned(perm).unwrap();
            assert_eq!(best, winner);
        }
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn route_with_zero_quote_is_accepted_but_ranked_last_among_same_hops() {
        let zero_quote = route(&[pool(0x01)], 0);
        let positive_quote = route(&[pool(0x02)], 1);
        let candidates = [zero_quote.clone(), positive_quote.clone()];
        let best = select_oracle_path(&candidates).unwrap();
        assert_eq!(best, &positive_quote);
    }

    #[test]
    fn all_candidates_with_zero_quote_selects_lexicographically_first() {
        let a = route(&[pool(0x01)], 0);
        let b = route(&[pool(0x02)], 0);
        let candidates = [b.clone(), a.clone()];
        let best = select_oracle_path(&candidates).unwrap();
        assert_eq!(best, &a);
    }
}
