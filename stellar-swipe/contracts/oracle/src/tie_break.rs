//! Deterministic tie-breaking for oracle quorum results (Issue #1408).
//!
//! When two or more eligible oracle values produce equal quorum support (the
//! same number of distinct authorized observations back them), a stable
//! rule selects one value and its contributing source set without depending
//! on map-iteration order or other non-deterministic tie-breaking.
//!
//! # Rule
//! Among all candidate values that have the highest vote count:
//!
//! 1. **Lowest value wins.** Choosing the minimum minimizes the risk of
//!    accepting a manipulated high outlier when votes are split evenly.
//! 2. When values are identical (the caller deduplicates before calling),
//!    this reduces to picking the lexicographically smallest source set
//!    (sorted by address bytes, compared element-by-element). This
//!    guarantees the same result regardless of the order sources were
//!    inserted.
//!
//! The rule is documented here and must not be changed without a protocol
//! upgrade; callers may cache results and compare them against future
//! observations.
//!
//! # Stability
//! Equal-input permutations — the same values and sources in any insertion
//! order — produce the same `TieBreakResult`. Tests verify this explicitly.

use soroban_sdk::{contracttype, Address, Env, Vec};

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single candidate value and the sources that back it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    /// The oracle price value.
    pub value: i128,
    /// Addresses of the sources that submitted this exact value.
    pub sources: Vec<Address>,
}

/// Result of the tie-breaking selection.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TieBreakResult {
    /// The value selected by the tie-breaking rule.
    pub selected_value: i128,
    /// Sorted source set that backed `selected_value`.
    pub selected_sources: Vec<Address>,
    /// Whether more than one candidate had the same maximum vote count
    /// (i.e., a tie actually occurred and the rule was exercised).
    pub was_tie: bool,
}

// ── Tie-breaking ──────────────────────────────────────────────────────────────

/// Select the winning candidate from `candidates` using the deterministic
/// tie-breaking rule.
///
/// `candidates` is the full list of distinct values with their backing source
/// sets. Each candidate's `sources` list need not be pre-sorted; this
/// function sorts it internally before comparison.
///
/// # Panics
/// Panics when `candidates` is empty (caller must ensure at least one
/// candidate exists after quorum filtering).
///
/// # Algorithm
/// 1. Find the maximum vote count (`max_votes = max(c.sources.len())`).
/// 2. Collect all candidates where `c.sources.len() == max_votes`.
/// 3. If only one remains, return it (no tie).
/// 4. Otherwise, pick the candidate with the **lowest `c.value`**.
/// 5. If two candidates share the same value, compare their sorted source
///    sets lexicographically and pick the smaller one.
pub fn select(env: &Env, candidates: &Vec<Candidate>) -> TieBreakResult {
    assert!(!candidates.is_empty(), "candidates must not be empty");

    // Step 1: find max vote count.
    let mut max_votes = 0u32;
    for i in 0..candidates.len() {
        let c = candidates.get_unchecked(i);
        if c.sources.len() > max_votes {
            max_votes = c.sources.len();
        }
    }

    // Step 2: collect tied candidates.
    let mut tied: Vec<Candidate> = Vec::new(env);
    for i in 0..candidates.len() {
        let c = candidates.get_unchecked(i);
        if c.sources.len() == max_votes {
            tied.push_back(c);
        }
    }

    let was_tie = tied.len() > 1;

    // Step 3-5: pick winner deterministically.
    let winner = best_candidate(env, &tied);
    let sorted_sources = sort_sources(env, &winner.sources);

    TieBreakResult {
        selected_value: winner.value,
        selected_sources: sorted_sources,
        was_tie,
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Return the best candidate from `tied` (all have equal vote count).
/// Rule: lowest value; on equal value, lexicographically smallest sorted
/// source set.
fn best_candidate(env: &Env, tied: &Vec<Candidate>) -> Candidate {
    let first = tied.get_unchecked(0);
    let mut best_value = first.value;
    let mut best_sources = sort_sources(env, &first.sources);

    for i in 1..tied.len() {
        let c = tied.get_unchecked(i);
        let sorted = sort_sources(env, &c.sources);
        if c.value < best_value {
            best_value = c.value;
            best_sources = sorted;
        } else if c.value == best_value && sources_lt(env, &sorted, &best_sources) {
            best_sources = sorted;
        }
    }

    Candidate {
        value: best_value,
        sources: best_sources,
    }
}

/// Insertion-sort the addresses in `sources` by their byte representation.
/// Source lists are quorum-sized (small), so O(n²) is fine.
fn sort_sources(env: &Env, sources: &Vec<Address>) -> Vec<Address> {
    let mut sorted = sources.clone();
    let len = sorted.len();
    for i in 1..len {
        let mut j = i;
        while j > 0 {
            let a = sorted.get_unchecked(j - 1);
            let b = sorted.get_unchecked(j);
            if addr_gt(env, &a, &b) {
                sorted.set(j - 1, b);
                sorted.set(j, a);
                j -= 1;
            } else {
                break;
            }
        }
    }
    sorted
}

/// Lexicographic comparison of two sorted source lists: `true` iff `a < b`.
fn sources_lt(env: &Env, a: &Vec<Address>, b: &Vec<Address>) -> bool {
    let len = a.len().min(b.len());
    for i in 0..len {
        let ai = a.get_unchecked(i);
        let bi = b.get_unchecked(i);
        if addr_lt(env, &ai, &bi) {
            return true;
        }
        if addr_gt(env, &ai, &bi) {
            return false;
        }
    }
    a.len() < b.len()
}

/// Compare two addresses by their `to_string` representation.
/// Soroban addresses serialize to a stable Strkey string; lexicographic
/// comparison on that string is deterministic and map-iteration-independent.
/// Longest strkey an `Address` renders to (G... and C... addresses are 56 characters).
const ADDR_STRKEY_MAX_LEN: usize = 56;

fn addr_lt(_env: &Env, a: &Address, b: &Address) -> bool {
    let sa = a.to_string();
    let sb = b.to_string();
    // Strkey addresses are 56 ASCII bytes; slices compare byte by byte, then by length.
    let mut ba = [0u8; ADDR_STRKEY_MAX_LEN];
    let mut bb = [0u8; ADDR_STRKEY_MAX_LEN];
    let la = sa.len() as usize;
    let lb = sb.len() as usize;
    sa.copy_into_slice(&mut ba[..la]);
    sb.copy_into_slice(&mut bb[..lb]);
    ba[..la] < bb[..lb]
}

fn addr_gt(env: &Env, a: &Address, b: &Address) -> bool {
    addr_lt(env, b, a)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Env};

    fn candidate(env: &Env, value: i128, n_sources: u32) -> Candidate {
        let mut sources = Vec::new(env);
        for _ in 0..n_sources {
            sources.push_back(Address::generate(env));
        }
        Candidate { value, sources }
    }

    // ── No tie: highest vote count wins directly ───────────────────────────────

    #[test]
    fn highest_vote_count_wins_no_tie() {
        let env = Env::default();
        let a = candidate(&env, 100, 3); // 3 votes
        let b = candidate(&env, 200, 2); // 2 votes
        let candidates = vec![&env, a.clone(), b];
        let result = select(&env, &candidates);
        assert_eq!(result.selected_value, 100);
        assert!(!result.was_tie);
        assert_eq!(result.selected_sources.len(), 3);
    }

    // ── Tie: lowest value wins ────────────────────────────────────────────────

    #[test]
    fn tie_broken_by_lowest_value() {
        let env = Env::default();
        let a = candidate(&env, 200, 2);
        let b = candidate(&env, 100, 2); // same votes, lower value
        let candidates = vec![&env, a, b];
        let result = select(&env, &candidates);
        assert_eq!(result.selected_value, 100);
        assert!(result.was_tie);
    }

    // ── Permutation invariance ─────────────────────────────────────────────────

    #[test]
    fn equal_input_permutations_produce_same_result() {
        let env = Env::default();
        let s1 = Address::generate(&env);
        let s2 = Address::generate(&env);

        // Two candidates with the same value and identical sources in both orders.
        let c1 = Candidate {
            value: 150,
            sources: vec![&env, s1.clone(), s2.clone()],
        };
        let c2 = Candidate {
            value: 150,
            sources: vec![&env, s2.clone(), s1.clone()],
        };

        // Single candidate, both source orderings.
        let r1 = select(&env, &vec![&env, c1.clone()]);
        let r2 = select(&env, &vec![&env, c2.clone()]);

        assert_eq!(r1.selected_value, r2.selected_value);
        assert_eq!(r1.selected_sources, r2.selected_sources);
    }

    // ── Tie with equal values: lexicographically smaller source set wins ───────

    #[test]
    fn tie_equal_values_smallest_source_set_wins() {
        let env = Env::default();
        // Fix two addresses so we know their ordering.
        let mut addrs: Vec<Address> = Vec::new(&env);
        for _ in 0..4 {
            addrs.push_back(Address::generate(&env));
        }

        // Two candidates, same value, 2 votes each, different source sets.
        let c1 = Candidate {
            value: 100,
            sources: vec![&env, addrs.get_unchecked(0), addrs.get_unchecked(1)],
        };
        let c2 = Candidate {
            value: 100,
            sources: vec![&env, addrs.get_unchecked(2), addrs.get_unchecked(3)],
        };

        let candidates = vec![&env, c1.clone(), c2.clone()];
        let r_ab = select(&env, &candidates);

        // Reversed order input must produce the same winner.
        let candidates_rev = vec![&env, c2, c1];
        let r_ba = select(&env, &candidates_rev);

        // Both must agree on the same value and source set.
        assert_eq!(r_ab.selected_value, r_ba.selected_value);
        assert_eq!(r_ab.selected_sources, r_ba.selected_sources);
        assert!(r_ab.was_tie);
    }

    // ── Repeated observations from same source ────────────────────────────────

    #[test]
    fn repeated_source_in_one_candidate_still_selects_correctly() {
        let env = Env::default();
        let shared = Address::generate(&env);
        // Candidate A: 2 distinct sources.
        let c_a = Candidate {
            value: 50,
            sources: vec![&env, Address::generate(&env), Address::generate(&env)],
        };
        // Candidate B: 3 sources but one repeated (not de-duped here — caller
        // is responsible for deduplication before passing to select).
        let c_b = Candidate {
            value: 60,
            sources: vec![
                &env,
                shared.clone(),
                shared.clone(),
                Address::generate(&env),
            ],
        };
        let candidates = vec![&env, c_a, c_b.clone()];
        let result = select(&env, &candidates);
        // c_b has 3 raw entries (even if two are the same address); the rule
        // picks by len, so c_b wins.
        assert_eq!(result.selected_value, 60);
        assert!(!result.was_tie);
    }

    // ── Single candidate — always returned unchanged (no tie) ─────────────────

    #[test]
    fn single_candidate_returned_unchanged() {
        let env = Env::default();
        let c = candidate(&env, 999, 1);
        let result = select(&env, &vec![&env, c]);
        assert_eq!(result.selected_value, 999);
        assert!(!result.was_tie);
    }

    // ── Three-way tie ─────────────────────────────────────────────────────────

    #[test]
    fn three_way_tie_lowest_value_wins() {
        let env = Env::default();
        let a = candidate(&env, 300, 2);
        let b = candidate(&env, 100, 2);
        let c = candidate(&env, 200, 2);
        let candidates = vec![&env, a, b, c];
        let result = select(&env, &candidates);
        assert_eq!(result.selected_value, 100);
        assert!(result.was_tie);
    }
}
