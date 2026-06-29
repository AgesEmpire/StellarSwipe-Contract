//! Cohort retention analysis for signal followers (Issue #672).
//!
//! Groups followers into cohorts by the week they first followed a provider,
//! and tracks what fraction of each cohort is still actively copy-trading
//! at +1, +4, and +12 weeks.

use soroban_sdk::{contracttype, Address, Env};

const SECONDS_PER_WEEK: u64 = 7 * 24 * 3600;

// ── Types ─────────────────────────────────────────────────────────────────────

/// A cohort is identified by the week-slot of first-follow.
/// `week_slot` = first_follow_timestamp / SECONDS_PER_WEEK.
#[contracttype]
#[derive(Clone, Debug)]
pub struct CohortData {
    /// Total followers who joined in this cohort week.
    pub total_followers: u32,
    /// Followers still active at +1 week (i.e., recorded activity within week 1-2).
    pub active_at_week_1: u32,
    /// Followers still active at +4 weeks.
    pub active_at_week_4: u32,
    /// Followers still active at +12 weeks.
    pub active_at_week_12: u32,
}

/// Summary returned by `get_cohort_retention`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct CohortRetention {
    pub week_slot: u64,
    pub total_followers: u32,
    /// Retention rate at +1 week in basis points (10_000 = 100 %).
    pub retention_bps_week_1: u32,
    /// Retention rate at +4 weeks in basis points.
    pub retention_bps_week_4: u32,
    /// Retention rate at +12 weeks in basis points.
    pub retention_bps_week_12: u32,
}

#[contracttype]
#[derive(Clone)]
enum CohortKey {
    /// (provider, week_slot) -> CohortData
    Cohort(Address, u64),
    /// (provider, user) -> week_slot of first follow
    FirstFollow(Address, Address),
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn week_slot(ts: u64) -> u64 {
    ts / SECONDS_PER_WEEK
}

fn load_cohort(env: &Env, provider: &Address, slot: u64) -> CohortData {
    env.storage()
        .persistent()
        .get(&CohortKey::Cohort(provider.clone(), slot))
        .unwrap_or(CohortData {
            total_followers: 0,
            active_at_week_1: 0,
            active_at_week_4: 0,
            active_at_week_12: 0,
        })
}

fn save_cohort(env: &Env, provider: &Address, slot: u64, data: &CohortData) {
    env.storage()
        .persistent()
        .set(&CohortKey::Cohort(provider.clone(), slot), data);
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Called when a user first follows a provider.
/// Records the cohort entry for the user.
pub fn record_follow(env: &Env, provider: &Address, user: &Address) {
    let key = CohortKey::FirstFollow(provider.clone(), user.clone());
    if env.storage().persistent().has(&key) {
        return; // already recorded
    }
    let now = env.ledger().timestamp();
    let slot = week_slot(now);
    env.storage().persistent().set(&key, &slot);

    let mut data = load_cohort(env, provider, slot);
    data.total_followers = data.total_followers.saturating_add(1);
    save_cohort(env, provider, slot, &data);
}

/// Called when a user copy-trades a signal from a provider.
/// Updates active-at-week-N buckets for their cohort.
pub fn record_activity(env: &Env, provider: &Address, user: &Address) {
    let key = CohortKey::FirstFollow(provider.clone(), user.clone());
    let slot: u64 = match env.storage().persistent().get(&key) {
        Some(s) => s,
        None => return, // user hasn't formally followed; skip
    };

    let now = env.ledger().timestamp();
    let current_slot = week_slot(now);
    let weeks_since = current_slot.saturating_sub(slot);

    let mut data = load_cohort(env, provider, slot);

    // Accumulate into the highest applicable bucket (not mutually exclusive —
    // being active at week 12 implies active at 4 and 1 too).
    if weeks_since >= 12 {
        data.active_at_week_12 = data.active_at_week_12.saturating_add(1);
        data.active_at_week_4 = data.active_at_week_4.saturating_add(1);
        data.active_at_week_1 = data.active_at_week_1.saturating_add(1);
    } else if weeks_since >= 4 {
        data.active_at_week_4 = data.active_at_week_4.saturating_add(1);
        data.active_at_week_1 = data.active_at_week_1.saturating_add(1);
    } else if weeks_since >= 1 {
        data.active_at_week_1 = data.active_at_week_1.saturating_add(1);
    }

    save_cohort(env, provider, slot, &data);
}

/// Read-only: return cohort retention summary for `(provider, week_slot)`.
pub fn get_cohort_retention(env: &Env, provider: &Address, cohort_week_slot: u64) -> CohortRetention {
    let data = load_cohort(env, provider, cohort_week_slot);
    let total = data.total_followers;

    let bps = |active: u32| -> u32 {
        if total == 0 {
            0
        } else {
            (active * 10_000) / total
        }
    };

    CohortRetention {
        week_slot: cohort_week_slot,
        total_followers: total,
        retention_bps_week_1: bps(data.active_at_week_1),
        retention_bps_week_4: bps(data.active_at_week_4),
        retention_bps_week_12: bps(data.active_at_week_12),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let provider = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        (env, provider, user1, user2)
    }

    #[test]
    fn test_new_follower_joins_cohort() {
        let (env, provider, user1, _) = setup();
        env.ledger().with_mut(|l| l.timestamp = SECONDS_PER_WEEK * 5);

        record_follow(&env, &provider, &user1);

        let retention = get_cohort_retention(&env, &provider, 5);
        assert_eq!(retention.total_followers, 1);
        assert_eq!(retention.retention_bps_week_1, 0); // no activity yet
    }

    #[test]
    fn test_activity_at_week_1_recorded() {
        let (env, provider, user1, _) = setup();
        env.ledger().with_mut(|l| l.timestamp = SECONDS_PER_WEEK * 10);
        record_follow(&env, &provider, &user1);

        // Advance 1 week
        env.ledger().with_mut(|l| l.timestamp = SECONDS_PER_WEEK * 11);
        record_activity(&env, &provider, &user1);

        let retention = get_cohort_retention(&env, &provider, 10);
        assert_eq!(retention.total_followers, 1);
        assert_eq!(retention.retention_bps_week_1, 10_000); // 100 %
        assert_eq!(retention.retention_bps_week_4, 0);
        assert_eq!(retention.retention_bps_week_12, 0);
    }

    #[test]
    fn test_activity_at_week_12_fills_all_buckets() {
        let (env, provider, user1, _) = setup();
        env.ledger().with_mut(|l| l.timestamp = SECONDS_PER_WEEK * 0);
        record_follow(&env, &provider, &user1);

        // Advance 12 weeks
        env.ledger().with_mut(|l| l.timestamp = SECONDS_PER_WEEK * 12);
        record_activity(&env, &provider, &user1);

        let retention = get_cohort_retention(&env, &provider, 0);
        assert_eq!(retention.retention_bps_week_1, 10_000);
        assert_eq!(retention.retention_bps_week_4, 10_000);
        assert_eq!(retention.retention_bps_week_12, 10_000);
    }

    #[test]
    fn test_multi_cohort_two_users() {
        let (env, provider, user1, user2) = setup();

        // user1 follows in week 0
        env.ledger().with_mut(|l| l.timestamp = 0);
        record_follow(&env, &provider, &user1);

        // user2 follows in week 1
        env.ledger().with_mut(|l| l.timestamp = SECONDS_PER_WEEK);
        record_follow(&env, &provider, &user2);

        // user1 is active at week 1 (relative to their cohort)
        env.ledger().with_mut(|l| l.timestamp = SECONDS_PER_WEEK + 1);
        record_activity(&env, &provider, &user1);

        // cohort 0: user1 joined, 100% at week 1
        let c0 = get_cohort_retention(&env, &provider, 0);
        assert_eq!(c0.total_followers, 1);
        assert_eq!(c0.retention_bps_week_1, 10_000);

        // cohort 1: user2 joined, 0% activity yet
        let c1 = get_cohort_retention(&env, &provider, 1);
        assert_eq!(c1.total_followers, 1);
        assert_eq!(c1.retention_bps_week_1, 0);
    }

    #[test]
    fn test_no_double_count_follow() {
        let (env, provider, user1, _) = setup();
        env.ledger().with_mut(|l| l.timestamp = 0);
        record_follow(&env, &provider, &user1);
        record_follow(&env, &provider, &user1); // idempotent

        let retention = get_cohort_retention(&env, &provider, 0);
        assert_eq!(retention.total_followers, 1);
    }

    #[test]
    fn test_empty_cohort_returns_zero() {
        let (env, provider, _, _) = setup();
        let retention = get_cohort_retention(&env, &provider, 99);
        assert_eq!(retention.total_followers, 0);
        assert_eq!(retention.retention_bps_week_1, 0);
    }
}
