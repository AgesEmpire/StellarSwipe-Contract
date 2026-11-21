//! Issue #515: Comprehensive audit logging for compliance.
//!
//! All contract operations are logged with an immutable, append-only audit trail.
//! Logs are stored in persistent storage keyed by a monotonic counter.
//! Compliance reports aggregate logs by operation type and time range.
//! Data retention: logs older than RETENTION_SECS are eligible for archival.

use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};

/// Retention window: 90 days in seconds
pub const RETENTION_SECS: u64 = 90 * 24 * 60 * 60;
/// Max logs returned in a single compliance report query
pub const MAX_REPORT_ENTRIES: u32 = 100;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditOperation {
    SignalCreated,
    SignalClosed,
    TradeExecuted,
    StakeChanged,
    ProviderBanned,
    FeeCollected,
    AdminAction,
    VoteCast,
    DisputeOpened,
    DisputeResolved,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AuditEntry {
    pub id: u64,
    pub operation: AuditOperation,
    pub actor: Address,
    pub timestamp: u64,
    /// Compact detail string (e.g. signal_id, amount, reason hash)
    pub detail: String,
}

#[contracttype]
#[derive(Clone)]
pub enum AuditKey {
    /// Monotonic counter for audit log IDs
    Counter,
    /// id -> AuditEntry
    Entry(u64),
    /// Oldest retained log ID (for archival cursor)
    ArchiveCursor,
}

/// Append an audit entry. Returns the assigned log ID.
pub fn log(
    env: &Env,
    operation: AuditOperation,
    actor: Address,
    detail: String,
) -> u64 {
    let id: u64 = env
        .storage()
        .persistent()
        .get(&AuditKey::Counter)
        .unwrap_or(0)
        + 1;
    env.storage().persistent().set(&AuditKey::Counter, &id);

    let entry = AuditEntry {
        id,
        operation: operation.clone(),
        actor: actor.clone(),
        timestamp: env.ledger().timestamp(),
        detail,
    };
    env.storage()
        .persistent()
        .set(&AuditKey::Entry(id), &entry);

    // Emit event for off-chain indexers
    env.events().publish(
        (Symbol::new(env, "audit_log"), actor),
        (id, operation),
    );

    id
}

/// Retrieve a single audit entry by ID.
pub fn get_entry(env: &Env, id: u64) -> Option<AuditEntry> {
    env.storage().persistent().get(&AuditKey::Entry(id))
}

/// Get the current audit log counter (total entries ever written).
pub fn get_log_count(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&AuditKey::Counter)
        .unwrap_or(0)
}

/// Compliance report: return up to MAX_REPORT_ENTRIES entries in [from_id, to_id].
/// Caller should paginate by advancing from_id = last_returned_id + 1.
pub fn compliance_report(env: &Env, from_id: u64, to_id: u64) -> Vec<AuditEntry> {
    let mut results = Vec::new(env);
    let end = to_id.min(from_id + MAX_REPORT_ENTRIES as u64 - 1);
    for id in from_id..=end {
        if let Some(entry) = get_entry(env, id) {
            results.push_back(entry);
        }
    }
    results
}

/// Archive (delete) audit entries older than RETENTION_SECS.
/// Processes up to `batch_size` entries starting from the archive cursor.
/// Returns the number of entries archived.
pub fn archive_old_entries(env: &Env, batch_size: u32) -> u32 {
    let now = env.ledger().timestamp();
    let cutoff = now.saturating_sub(RETENTION_SECS);
    let cursor: u64 = env
        .storage()
        .persistent()
        .get(&AuditKey::ArchiveCursor)
        .unwrap_or(1);
    let total = get_log_count(env);

    let mut archived = 0u32;
    let mut next_cursor = cursor;

    for id in cursor..=total {
        if archived >= batch_size {
            break;
        }
        if let Some(entry) = get_entry(env, id) {
            if entry.timestamp < cutoff {
                env.storage().persistent().remove(&AuditKey::Entry(id));
                archived += 1;
                next_cursor = id + 1;
            } else {
                // Entries are ordered by time; stop once we hit a fresh one
                break;
            }
        }
    }

    env.storage()
        .persistent()
        .set(&AuditKey::ArchiveCursor, &next_cursor);
    archived
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SignalRegistry;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    fn with_registry<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = Env::default();
        env.ledger().set_timestamp(1_000_000);
        #[allow(deprecated)]
        let cid = env.register_contract(None, SignalRegistry);
        env.as_contract(&cid, || f(&env))
    }

    #[test]
    fn test_log_creates_entry() {
        with_registry(|env| {
            let actor = Address::generate(env);
            let id = log(
                env,
                AuditOperation::SignalCreated,
                actor.clone(),
                String::from_str(env, "signal_id=1"),
            );
            assert_eq!(id, 1);
            let entry = get_entry(env, 1).unwrap();
            assert_eq!(entry.id, 1);
            assert_eq!(entry.operation, AuditOperation::SignalCreated);
            assert_eq!(entry.actor, actor);
        });
    }

    #[test]
    fn test_log_counter_increments() {
        with_registry(|env| {
            let actor = Address::generate(env);
            for i in 1u64..=5 {
                let id = log(
                    env,
                    AuditOperation::TradeExecuted,
                    actor.clone(),
                    String::from_str(env, "x"),
                );
                assert_eq!(id, i);
            }
            assert_eq!(get_log_count(env), 5);
        });
    }

    #[test]
    fn test_compliance_report_range() {
        with_registry(|env| {
            let actor = Address::generate(env);
            for _ in 0..10u32 {
                log(env, AuditOperation::FeeCollected, actor.clone(), String::from_str(env, "x"));
            }
            let report = compliance_report(env, 3, 7);
            assert_eq!(report.len(), 5);
            assert_eq!(report.get(0).unwrap().id, 3);
            assert_eq!(report.get(4).unwrap().id, 7);
        });
    }

    #[test]
    fn test_compliance_report_capped_at_max() {
        with_registry(|env| {
            let actor = Address::generate(env);
            for _ in 0..150u32 {
                log(env, AuditOperation::AdminAction, actor.clone(), String::from_str(env, "x"));
            }
            let report = compliance_report(env, 1, 150);
            assert_eq!(report.len(), MAX_REPORT_ENTRIES);
        });
    }

    #[test]
    fn test_archive_removes_old_entries() {
        with_registry(|env| {
            let actor = Address::generate(env);
            // Log 5 entries at t=1_000_000
            for _ in 0..5u32 {
                log(env, AuditOperation::StakeChanged, actor.clone(), String::from_str(env, "x"));
            }
            // Advance time past retention window
            env.ledger().set_timestamp(1_000_000 + RETENTION_SECS + 1);
            let archived = archive_old_entries(env, 10);
            assert_eq!(archived, 5);
            assert!(get_entry(env, 1).is_none());
        });
    }

    #[test]
    fn test_archive_respects_batch_size() {
        with_registry(|env| {
            let actor = Address::generate(env);
            for _ in 0..10u32 {
                log(env, AuditOperation::VoteCast, actor.clone(), String::from_str(env, "x"));
            }
            env.ledger().set_timestamp(1_000_000 + RETENTION_SECS + 1);
            let archived = archive_old_entries(env, 3);
            assert_eq!(archived, 3);
            // Entries 1-3 gone, 4-10 still present
            assert!(get_entry(env, 1).is_none());
            assert!(get_entry(env, 4).is_some());
        });
    }
}
