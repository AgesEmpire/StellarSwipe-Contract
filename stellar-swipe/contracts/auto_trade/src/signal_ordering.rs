//! Deterministic ordering and conflict resolution for concurrent auto-trade signals.
//!
//! When multiple signals arrive within the same ledger (same timestamp), their
//! relative processing order must be stable and reproducible so that repeated
//! execution produces the same state.
//!
//! ## Ordering strategy
//!
//! Signals are ranked by a two-level key:
//! 1. **Timestamp** (ascending) – earlier-created signals execute first.
//! 2. **Signal ID** (ascending) – tie-breaker when timestamps are equal.
//!
//! This guarantees a total order over any set of signals and makes execution
//! deterministic regardless of network arrival order.
//!
//! ## Conflict resolution
//!
//! Two signals are considered *conflicting* when they target the same asset
//! pair but have opposing directions (e.g., BUY vs SELL).  The winning signal
//! is chosen by the same ordering key: lower timestamp wins; on a tie, lower
//! signal ID wins.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Env, Vec};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Compact representation of a pending signal used for ordering.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalOrder {
    /// Unique monotonic identifier assigned at signal creation time.
    pub signal_id: u64,
    /// Ledger timestamp at which the signal was submitted.
    pub timestamp: u64,
    /// Asset-pair identifier (opaque u32 matching the on-chain asset registry).
    pub asset_pair: u32,
    /// Direction: `true` = BUY, `false` = SELL.
    pub is_buy: bool,
}

// ── Ordering ──────────────────────────────────────────────────────────────────

/// Return the *ordering key* for a [`SignalOrder`].
///
/// The key is a `(timestamp, signal_id)` tuple.  Because both values are
/// monotonically increasing, lexicographic comparison of the tuple produces
/// a deterministic total order.
#[inline]
pub fn ordering_key(s: &SignalOrder) -> (u64, u64) {
    (s.timestamp, s.signal_id)
}

/// Sort `signals` in-place using the deterministic ordering key.
///
/// This is an insertion-sort over the `soroban_sdk::Vec` (no `std` allocator).
/// For typical batch sizes (< 100 signals per ledger) the cost is acceptable.
pub fn sort_signals(signals: &mut Vec<SignalOrder>) {
    let n = signals.len();
    if n <= 1 {
        return;
    }
    // Insertion sort: O(n²) but avoids heap allocation in no_std.
    for i in 1..n {
        let mut j = i;
        while j > 0 {
            let a = signals.get_unchecked(j - 1);
            let b = signals.get_unchecked(j);
            if ordering_key(&a) > ordering_key(&b) {
                signals.set(j - 1, b);
                signals.set(j, a);
                j -= 1;
            } else {
                break;
            }
        }
    }
}

// ── Conflict resolution ───────────────────────────────────────────────────────

/// Return `true` when two signals conflict (same asset pair, opposite direction).
#[inline]
pub fn are_conflicting(a: &SignalOrder, b: &SignalOrder) -> bool {
    a.asset_pair == b.asset_pair && a.is_buy != b.is_buy
}

/// Resolve a conflict between two signals: returns a reference to the winner.
///
/// The winner is the signal with the lower `(timestamp, signal_id)` key.
pub fn resolve_conflict<'a>(a: &'a SignalOrder, b: &'a SignalOrder) -> &'a SignalOrder {
    if ordering_key(a) <= ordering_key(b) {
        a
    } else {
        b
    }
}

/// From a slice of [`SignalOrder`]s, remove signals that lose a conflict.
///
/// A signal is **removed** when there exists another signal with the same
/// `asset_pair`, the opposite direction, and a strictly lower ordering key.
///
/// Returns a new `Vec` containing only the winning (non-conflicted) signals,
/// already sorted by the deterministic ordering key.
pub fn deconflict_signals(env: &Env, mut signals: Vec<SignalOrder>) -> Vec<SignalOrder> {
    sort_signals(&mut signals);

    let mut result: Vec<SignalOrder> = Vec::new(env);

    'outer: for i in 0..signals.len() {
        let candidate = signals.get_unchecked(i);
        // Because the list is sorted, any earlier entry that conflicts with
        // `candidate` has a lower (or equal) ordering key and therefore wins.
        for j in 0..i {
            let earlier = signals.get_unchecked(j);
            if are_conflicting(&earlier, &candidate) {
                // `earlier` dominates `candidate`; skip candidate.
                continue 'outer;
            }
        }
        result.push_back(candidate);
    }

    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn env() -> Env {
        Env::default()
    }

    fn sig(id: u64, ts: u64, pair: u32, buy: bool) -> SignalOrder {
        SignalOrder {
            signal_id: id,
            timestamp: ts,
            asset_pair: pair,
            is_buy: buy,
        }
    }

    // ── ordering_key ──────────────────────────────────────────────────────

    #[test]
    fn ordering_key_uses_timestamp_then_id() {
        let a = sig(10, 100, 1, true);
        let b = sig(1, 200, 1, true);
        // a has lower timestamp → a comes first
        assert!(ordering_key(&a) < ordering_key(&b));
    }

    #[test]
    fn ordering_key_tiebreak_by_signal_id() {
        let a = sig(1, 100, 1, true);
        let b = sig(2, 100, 1, true);
        assert!(ordering_key(&a) < ordering_key(&b));
    }

    // ── sort_signals ──────────────────────────────────────────────────────

    #[test]
    fn sort_signals_produces_deterministic_order() {
        let e = env();
        let mut signals = Vec::new(&e);
        signals.push_back(sig(5, 300, 1, true));
        signals.push_back(sig(1, 100, 1, true));
        signals.push_back(sig(3, 200, 1, true));
        sort_signals(&mut signals);
        assert_eq!(signals.get_unchecked(0).signal_id, 1);
        assert_eq!(signals.get_unchecked(1).signal_id, 3);
        assert_eq!(signals.get_unchecked(2).signal_id, 5);
    }

    #[test]
    fn sort_signals_stable_on_same_timestamp() {
        let e = env();
        let mut signals = Vec::new(&e);
        signals.push_back(sig(3, 100, 1, true));
        signals.push_back(sig(1, 100, 1, true));
        signals.push_back(sig(2, 100, 1, true));
        sort_signals(&mut signals);
        assert_eq!(signals.get_unchecked(0).signal_id, 1);
        assert_eq!(signals.get_unchecked(1).signal_id, 2);
        assert_eq!(signals.get_unchecked(2).signal_id, 3);
    }

    #[test]
    fn sort_signals_single_element_unchanged() {
        let e = env();
        let mut signals = Vec::new(&e);
        signals.push_back(sig(42, 999, 1, true));
        sort_signals(&mut signals);
        assert_eq!(signals.get_unchecked(0).signal_id, 42);
    }

    // ── conflict detection ────────────────────────────────────────────────

    #[test]
    fn same_pair_opposite_direction_conflicts() {
        let a = sig(1, 100, 7, true);
        let b = sig(2, 100, 7, false);
        assert!(are_conflicting(&a, &b));
    }

    #[test]
    fn same_pair_same_direction_no_conflict() {
        let a = sig(1, 100, 7, true);
        let b = sig(2, 100, 7, true);
        assert!(!are_conflicting(&a, &b));
    }

    #[test]
    fn different_pairs_opposite_direction_no_conflict() {
        let a = sig(1, 100, 1, true);
        let b = sig(2, 100, 2, false);
        assert!(!are_conflicting(&a, &b));
    }

    // ── resolve_conflict ──────────────────────────────────────────────────

    #[test]
    fn earlier_timestamp_wins_conflict() {
        let a = sig(1, 100, 1, true);
        let b = sig(2, 200, 1, false);
        assert_eq!(resolve_conflict(&a, &b).signal_id, 1);
    }

    #[test]
    fn lower_id_wins_on_timestamp_tie() {
        let a = sig(1, 100, 1, true);
        let b = sig(2, 100, 1, false);
        assert_eq!(resolve_conflict(&a, &b).signal_id, 1);
    }

    // ── deconflict_signals ────────────────────────────────────────────────

    #[test]
    fn no_conflicts_all_signals_kept() {
        let e = env();
        let mut signals = Vec::new(&e);
        signals.push_back(sig(1, 100, 1, true));
        signals.push_back(sig(2, 200, 2, false));
        signals.push_back(sig(3, 300, 3, true));
        let result = deconflict_signals(&e, signals);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn conflicting_signal_loser_removed() {
        let e = env();
        let mut signals = Vec::new(&e);
        // Signal 1 (BUY, earlier) wins over signal 2 (SELL, later).
        signals.push_back(sig(2, 200, 5, false)); // loser
        signals.push_back(sig(1, 100, 5, true)); // winner
        let result = deconflict_signals(&e, signals);
        assert_eq!(result.len(), 1);
        assert_eq!(result.get_unchecked(0).signal_id, 1);
    }

    #[test]
    fn repeated_execution_same_result() {
        let e = env();
        let mut signals = Vec::new(&e);
        signals.push_back(sig(3, 300, 1, false));
        signals.push_back(sig(1, 100, 1, true)); // winner on pair 1
        signals.push_back(sig(2, 200, 2, true)); // no conflict

        let r1 = deconflict_signals(&e, signals.clone());
        let r2 = deconflict_signals(&e, signals);

        assert_eq!(r1.len(), r2.len());
        for i in 0..r1.len() {
            assert_eq!(r1.get_unchecked(i).signal_id, r2.get_unchecked(i).signal_id);
        }
    }
}
