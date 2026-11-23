//! Time-window boundary convention for every analytics aggregation (Issue #1378).
//!
//! # Convention: half-open intervals `[start, end)`
//!
//! Every time window in this contract **includes its start and excludes its
//! end**. A timestamp `t` is inside `[start, end)` iff `start <= t && t < end`.
//!
//! Consequences, applied uniformly:
//!
//! * **Adjacent windows tile without gaps or overlap.** `[a, b)` and `[b, c)`
//!   together cover exactly `[a, c)`; an event at exactly `b` is counted in the
//!   second window only. Summing consecutive windows therefore equals one
//!   query over their union.
//! * **Ingestion buckets.** An event with timestamp `t` is stored in bucket
//!   `k = t / BUCKET_SECONDS`, covering `[k * BUCKET_SECONDS, (k + 1) * BUCKET_SECONDS)`.
//!   An event on a bucket boundary belongs to the bucket that *starts* there.
//! * **Window queries** (`get_window_aggregate`) take `[start, end)` with both
//!   bounds aligned to `BUCKET_SECONDS`, so a window is always a whole number
//!   of buckets and the result never depends on where inside a bucket an event
//!   fell. `start == end` is the empty window. Unaligned or reversed bounds are
//!   rejected rather than silently rounded.
//! * **Durations** (rate limits, cache TTLs) are the window
//!   `[since, since + duration)`: a duration has *elapsed* at the first instant
//!   outside it, i.e. when `now >= since + duration`. So the weekly health
//!   report may be emitted again at exactly `last + SECONDS_PER_WEEK`, and a
//!   cache entry written at `cached_at` with TTL `ttl` is a hit for
//!   `now < cached_at + ttl` and a miss from `cached_at + ttl` onward.

/// Width of one ingestion bucket, in seconds (1 hour).
pub const BUCKET_SECONDS: u64 = 3_600;

/// Maximum buckets a single window query may span (7 days of hourly buckets),
/// bounding per-call storage reads.
pub const MAX_WINDOW_BUCKETS: u64 = 168;

/// `true` iff `t` lies in the half-open window `[start, end)`.
#[inline]
pub fn contains(start: u64, end: u64, t: u64) -> bool {
    start <= t && t < end
}

/// `true` once `now` has left the window `[since, since + duration)`.
///
/// Saturates at `u64::MAX`, so an overflowing deadline is treated as never
/// elapsing rather than wrapping to the past.
#[inline]
pub fn has_elapsed(since: u64, duration: u64, now: u64) -> bool {
    now >= since.saturating_add(duration)
}

/// Index of the bucket containing timestamp `t`.
#[inline]
pub fn bucket_of(t: u64) -> u64 {
    t / BUCKET_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open() {
        assert!(contains(10, 20, 10));
        assert!(contains(10, 20, 19));
        assert!(!contains(10, 20, 20));
        assert!(!contains(10, 20, 9));
        // Empty window contains nothing.
        assert!(!contains(10, 10, 10));
    }

    #[test]
    fn adjacent_windows_partition_every_timestamp() {
        let (a, b, c) = (0u64, 7u64, 15u64);
        for t in a..c {
            let hits = contains(a, b, t) as u32 + contains(b, c, t) as u32;
            assert_eq!(hits, 1, "t={t} must fall in exactly one window");
            assert!(contains(a, c, t));
        }
        assert!(!contains(a, b, c) && !contains(b, c, c));
    }

    #[test]
    fn has_elapsed_matches_window_exit() {
        assert!(!has_elapsed(100, 50, 100));
        assert!(!has_elapsed(100, 50, 149));
        assert!(has_elapsed(100, 50, 150));
        // Consistent with `contains`: elapsed exactly when outside the window.
        for now in 100..200 {
            assert_eq!(has_elapsed(100, 50, now), !contains(100, 150, now));
        }
        // Saturating deadline never elapses.
        assert!(!has_elapsed(u64::MAX - 1, 10, u64::MAX - 1));
    }

    #[test]
    fn bucket_boundaries_belong_to_the_starting_bucket() {
        assert_eq!(bucket_of(0), 0);
        assert_eq!(bucket_of(BUCKET_SECONDS - 1), 0);
        assert_eq!(bucket_of(BUCKET_SECONDS), 1);
        assert_eq!(bucket_of(2 * BUCKET_SECONDS - 1), 1);
    }
}
