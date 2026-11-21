//! Signal lifecycle extensions for the signal registry.
//!
//! * **Immutable signal revisions (issue #1140)** — every edit appends a new
//!   revision with a monotonic identifier and its creator. Stored revisions
//!   are never rewritten; only the signal head (which points at the current
//!   revision) changes. History is read in bounded pages.
//! * **Moderation appeals (issue #1141)** — a decided moderation case can be
//!   appealed once by the signal provider, reviewed by a registered appeal
//!   reviewer who is not the appellant, and resolved by that reviewer with a
//!   final reason. Appeals left unresolved past [`APPEAL_TIMEOUT_SECONDS`]
//!   can be expired by anyone and resolve as rejected.
//! * **Category tombstones (issue #1143)** — deleting a category tombstones
//!   its identifier permanently. The label is retained so historical signals
//!   stay readable, but new signals cannot target it and the identifier can
//!   never be (re-)imported with different semantics.
//! * **Reputation decay checkpoints (issue #1144)** — decay after long
//!   inactivity is advanced in bounded steps of at most
//!   [`MAX_DECAY_INTERVALS_PER_CALL`] intervals. Progress is persisted, and
//!   repeated calls converge to the value reported by `get_reputation`.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, String, Vec};

use crate::{
    DecaySchedule, ReputationRecord, SignalError, SignalRegistry, SignalRegistryArgs,
    SignalRegistryClient, ADMIN_KEY, REPUTATION_KEY, SCHEDULE_KEY,
};

/// Maximum number of revisions returned by one history page.
pub const MAX_REVISION_PAGE: u32 = 50;

/// Seconds an appeal may stay unresolved before anyone may expire it.
pub const APPEAL_TIMEOUT_SECONDS: u64 = 7 * 24 * 60 * 60;

/// Maximum number of decay intervals applied by one checkpoint call.
pub const MAX_DECAY_INTERVALS_PER_CALL: u64 = 32;

const SIGNAL_COUNT_KEY: &str = "signal_count";
const SIGNAL_KEY: &str = "signal";
const SIGNAL_REVISION_KEY: &str = "signal_rev";
const CATEGORY_KEY: &str = "category";
const CASE_COUNT_KEY: &str = "case_count";
const CASE_KEY: &str = "mod_case";
const APPEAL_KEY: &str = "appeal";
const REVIEWER_KEY: &str = "appeal_reviewer";
const DECAY_CHECKPOINT_KEY: &str = "decay_ckpt";

/// Mutable head of a signal; points at its current revision.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalHead {
    pub provider: Address,
    pub category: u32,
    pub current_revision: u32,
}

/// Immutable snapshot of a signal's content at one revision.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalRevision {
    pub revision: u32,
    pub creator: Address,
    pub content: String,
    pub created_at: u64,
}

/// Category record. `tombstoned` categories keep their label for historical
/// reads but can never be targeted or reused.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Category {
    pub label: String,
    pub tombstoned: bool,
    pub deleted_at: u64,
}

/// Lifecycle of a moderation case and its (single) appeal.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaseStatus {
    /// Moderation is still pending a decision; not appealable.
    Open,
    /// Moderation decided; the provider may appeal.
    Decided,
    /// An appeal has been submitted and awaits resolution.
    Appealed,
    /// The appeal was resolved (approved, rejected, or timed out).
    AppealResolved,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModerationCase {
    pub signal_id: u64,
    pub moderator: Address,
    pub reason: String,
    pub status: CaseStatus,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Appeal {
    pub appellant: Address,
    pub reason: String,
    pub submitted_at: u64,
    pub reviewer: Option<Address>,
    pub approved: Option<bool>,
    pub final_reason: Option<String>,
}

/// Event payload published when an appeal is resolved or expires.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppealResolvedEvent {
    pub case_id: u64,
    pub signal_id: u64,
    pub approved: bool,
    pub final_reason: String,
    pub resolver: Option<Address>,
    pub timed_out: bool,
}

/// Persisted progress of bounded reputation decay for one provider.
///
/// `anchor` is the `last_updated` of the reputation record the checkpoint
/// belongs to; a new reputation update invalidates the checkpoint.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecayCheckpoint {
    pub anchor: u64,
    pub intervals_applied: u64,
    pub score: i128,
    pub complete: bool,
}

#[contractimpl]
impl SignalRegistry {
    // ---------------------------------------------------------------------
    // Categories (issue #1143)
    // ---------------------------------------------------------------------

    /// Create (or import) a category. Fails for active identifiers and,
    /// permanently, for tombstoned identifiers.
    pub fn create_category(env: Env, id: u32, label: String) -> Result<(), SignalError> {
        require_admin(&env)?;
        if label.len() == 0 {
            return Err(SignalError::EmptyMetadata);
        }
        let key = (CATEGORY_KEY, id);
        if let Some(existing) = env.storage().persistent().get::<_, Category>(&key) {
            return Err(if existing.tombstoned {
                SignalError::CategoryTombstoned
            } else {
                SignalError::CategoryExists
            });
        }
        let category = Category { label, tombstoned: false, deleted_at: 0 };
        env.storage().persistent().set(&key, &category);
        Ok(())
    }

    /// Tombstone a category. Its label is retained for historical reads.
    pub fn delete_category(env: Env, id: u32) -> Result<(), SignalError> {
        require_admin(&env)?;
        let key = (CATEGORY_KEY, id);
        let mut category: Category = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(SignalError::CategoryNotFound)?;
        if category.tombstoned {
            return Err(SignalError::CategoryTombstoned);
        }
        category.tombstoned = true;
        category.deleted_at = env.ledger().timestamp();
        env.storage().persistent().set(&key, &category);
        env.events().publish((symbol_short!("cat_del"), id), category.label);
        Ok(())
    }

    pub fn get_category(env: Env, id: u32) -> Option<Category> {
        env.storage().persistent().get(&(CATEGORY_KEY, id))
    }

    // ---------------------------------------------------------------------
    // Signals and immutable revisions (issue #1140)
    // ---------------------------------------------------------------------

    /// Create a signal in an active category. Returns the new signal id; the
    /// initial content is stored as revision 1.
    pub fn create_signal(
        env: Env,
        provider: Address,
        category: u32,
        content: String,
    ) -> Result<u64, SignalError> {
        Self::require_valid_address(&env, &provider)?;
        provider.require_auth();
        Self::validate_metadata(&content)?;
        let cat: Category = env
            .storage()
            .persistent()
            .get(&(CATEGORY_KEY, category))
            .ok_or(SignalError::CategoryNotFound)?;
        if cat.tombstoned {
            return Err(SignalError::CategoryTombstoned);
        }

        let id: u64 = env.storage().instance().get(&SIGNAL_COUNT_KEY).unwrap_or(0) + 1;
        env.storage().instance().set(&SIGNAL_COUNT_KEY, &id);
        let head = SignalHead { provider: provider.clone(), category, current_revision: 1 };
        env.storage().persistent().set(&(SIGNAL_KEY, id), &head);
        store_revision(&env, id, 1, provider, content);
        Ok(id)
    }

    /// Edit a signal by appending a new revision. Earlier revisions are left
    /// untouched. Returns the new revision id.
    pub fn edit_signal(
        env: Env,
        provider: Address,
        signal_id: u64,
        content: String,
    ) -> Result<u32, SignalError> {
        provider.require_auth();
        Self::validate_metadata(&content)?;
        let key = (SIGNAL_KEY, signal_id);
        let mut head: SignalHead = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(SignalError::SignalNotFound)?;
        if head.provider != provider {
            return Err(SignalError::Unauthorized);
        }
        let revision = head
            .current_revision
            .checked_add(1)
            .ok_or(SignalError::MetadataRevisionOverflow)?;
        store_revision(&env, signal_id, revision, provider, content);
        head.current_revision = revision;
        env.storage().persistent().set(&key, &head);
        Ok(revision)
    }

    pub fn get_signal(env: Env, signal_id: u64) -> Option<SignalHead> {
        env.storage().persistent().get(&(SIGNAL_KEY, signal_id))
    }

    /// Label of the signal's category, retained even after tombstoning.
    pub fn get_signal_category_label(env: Env, signal_id: u64) -> Option<String> {
        let head = Self::get_signal(env.clone(), signal_id)?;
        Self::get_category(env, head.category).map(|c| c.label)
    }

    pub fn get_signal_revision(env: Env, signal_id: u64, revision: u32) -> Option<SignalRevision> {
        env.storage()
            .persistent()
            .get(&(SIGNAL_REVISION_KEY, signal_id, revision))
    }

    /// Page of revisions starting at `start` (1-based), oldest first. At most
    /// [`MAX_REVISION_PAGE`] entries are returned.
    pub fn get_signal_history(
        env: Env,
        signal_id: u64,
        start: u32,
        limit: u32,
    ) -> Result<Vec<SignalRevision>, SignalError> {
        if limit == 0 || limit > MAX_REVISION_PAGE {
            return Err(SignalError::InvalidBatchSize);
        }
        let head = Self::get_signal(env.clone(), signal_id).ok_or(SignalError::SignalNotFound)?;
        let mut out = Vec::new(&env);
        let first = start.max(1);
        let last = first.saturating_add(limit - 1).min(head.current_revision);
        let mut rev = first;
        while rev <= last {
            if let Some(r) = Self::get_signal_revision(env.clone(), signal_id, rev) {
                out.push_back(r);
            }
            rev += 1;
        }
        Ok(out)
    }

    // ---------------------------------------------------------------------
    // Moderation appeals (issue #1141)
    // ---------------------------------------------------------------------

    /// Admin registers or removes an appeal reviewer.
    pub fn set_appeal_reviewer(env: Env, reviewer: Address, enabled: bool) -> Result<(), SignalError> {
        Self::require_valid_address(&env, &reviewer)?;
        require_admin(&env)?;
        let key = (REVIEWER_KEY, reviewer);
        if enabled {
            env.storage().persistent().set(&key, &true);
        } else {
            env.storage().persistent().remove(&key);
        }
        Ok(())
    }

    /// Admin opens a moderation case against a signal.
    pub fn open_moderation_case(env: Env, signal_id: u64, reason: String) -> Result<u64, SignalError> {
        let moderator = require_admin(&env)?;
        if Self::get_signal(env.clone(), signal_id).is_none() {
            return Err(SignalError::SignalNotFound);
        }
        let id: u64 = env.storage().instance().get(&CASE_COUNT_KEY).unwrap_or(0) + 1;
        env.storage().instance().set(&CASE_COUNT_KEY, &id);
        let case = ModerationCase { signal_id, moderator, reason, status: CaseStatus::Open };
        env.storage().persistent().set(&(CASE_KEY, id), &case);
        Ok(id)
    }

    /// Admin records the moderation decision, making the case appealable.
    pub fn decide_moderation_case(env: Env, case_id: u64) -> Result<(), SignalError> {
        require_admin(&env)?;
        let mut case = load_case(&env, case_id)?;
        if case.status != CaseStatus::Open {
            return Err(SignalError::InvalidCaseState);
        }
        case.status = CaseStatus::Decided;
        env.storage().persistent().set(&(CASE_KEY, case_id), &case);
        Ok(())
    }

    /// Signal provider appeals a decided case. Unresolved (open) cases and
    /// cases whose appeal already exists or was resolved are rejected.
    pub fn submit_appeal(
        env: Env,
        appellant: Address,
        case_id: u64,
        reason: String,
    ) -> Result<(), SignalError> {
        appellant.require_auth();
        let mut case = load_case(&env, case_id)?;
        if case.status != CaseStatus::Decided {
            return Err(SignalError::InvalidCaseState);
        }
        let head = Self::get_signal(env.clone(), case.signal_id).ok_or(SignalError::SignalNotFound)?;
        if head.provider != appellant {
            return Err(SignalError::Unauthorized);
        }
        let appeal = Appeal {
            appellant,
            reason,
            submitted_at: env.ledger().timestamp(),
            reviewer: None,
            approved: None,
            final_reason: None,
        };
        env.storage().persistent().set(&(APPEAL_KEY, case_id), &appeal);
        case.status = CaseStatus::Appealed;
        env.storage().persistent().set(&(CASE_KEY, case_id), &case);
        Ok(())
    }

    /// A registered reviewer claims an appeal for review. The appellant can
    /// never review their own appeal.
    pub fn review_appeal(env: Env, reviewer: Address, case_id: u64) -> Result<(), SignalError> {
        reviewer.require_auth();
        require_reviewer(&env, &reviewer)?;
        let (_, mut appeal) = load_pending_appeal(&env, case_id)?;
        if appeal.appellant == reviewer {
            return Err(SignalError::SelfReview);
        }
        if appeal.reviewer.is_some() {
            return Err(SignalError::InvalidCaseState);
        }
        appeal.reviewer = Some(reviewer);
        env.storage().persistent().set(&(APPEAL_KEY, case_id), &appeal);
        Ok(())
    }

    /// The assigned reviewer resolves the appeal with a final reason.
    pub fn resolve_appeal(
        env: Env,
        reviewer: Address,
        case_id: u64,
        approved: bool,
        final_reason: String,
    ) -> Result<(), SignalError> {
        reviewer.require_auth();
        require_reviewer(&env, &reviewer)?;
        let (case, appeal) = load_pending_appeal(&env, case_id)?;
        if appeal.appellant == reviewer {
            return Err(SignalError::SelfReview);
        }
        if appeal.reviewer.as_ref() != Some(&reviewer) {
            return Err(SignalError::Unauthorized);
        }
        if env.ledger().timestamp() >= appeal.submitted_at.saturating_add(APPEAL_TIMEOUT_SECONDS) {
            return Err(SignalError::AppealExpired);
        }
        finish_appeal(&env, case_id, case, appeal, approved, final_reason, Some(reviewer), false);
        Ok(())
    }

    /// Anyone may expire an appeal left unresolved past the timeout; it
    /// resolves as rejected.
    pub fn expire_appeal(env: Env, case_id: u64) -> Result<(), SignalError> {
        let (case, appeal) = load_pending_appeal(&env, case_id)?;
        if env.ledger().timestamp() < appeal.submitted_at.saturating_add(APPEAL_TIMEOUT_SECONDS) {
            return Err(SignalError::AppealNotExpired);
        }
        let reason = String::from_str(&env, "appeal timed out");
        finish_appeal(&env, case_id, case, appeal, false, reason, None, true);
        Ok(())
    }

    pub fn get_moderation_case(env: Env, case_id: u64) -> Option<ModerationCase> {
        env.storage().persistent().get(&(CASE_KEY, case_id))
    }

    pub fn get_appeal(env: Env, case_id: u64) -> Option<Appeal> {
        env.storage().persistent().get(&(APPEAL_KEY, case_id))
    }

    // ---------------------------------------------------------------------
    // Bounded reputation decay checkpoints (issue #1144)
    // ---------------------------------------------------------------------

    /// Advance a provider's decay by at most [`MAX_DECAY_INTERVALS_PER_CALL`]
    /// intervals and persist the checkpoint. Repeated calls converge to the
    /// value reported by `get_reputation`; `complete` marks convergence.
    pub fn advance_reputation_decay(env: Env, provider: Address) -> Result<DecayCheckpoint, SignalError> {
        let schedule: DecaySchedule = env
            .storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)?;
        let record: ReputationRecord = env
            .storage()
            .persistent()
            .get(&(REPUTATION_KEY, provider.clone()))
            .ok_or(SignalError::InvalidReputationUpdate)?;
        let key = (DECAY_CHECKPOINT_KEY, provider);
        let mut checkpoint = match env.storage().persistent().get::<_, DecayCheckpoint>(&key) {
            Some(c) if c.anchor == record.last_updated => c,
            _ => DecayCheckpoint {
                anchor: record.last_updated,
                intervals_applied: 0,
                score: record.score,
                complete: false,
            },
        };

        let target = due_intervals(&record, env.ledger().timestamp(), &schedule);
        let next = target.min(checkpoint.intervals_applied.saturating_add(MAX_DECAY_INTERVALS_PER_CALL));
        checkpoint.intervals_applied = checkpoint.intervals_applied.max(next);
        checkpoint.score = decayed_score(&record, checkpoint.intervals_applied, &schedule);
        checkpoint.complete = checkpoint.intervals_applied >= target;
        env.storage().persistent().set(&key, &checkpoint);
        Ok(checkpoint)
    }

    pub fn get_decay_checkpoint(env: Env, provider: Address) -> Option<DecayCheckpoint> {
        env.storage().persistent().get(&(DECAY_CHECKPOINT_KEY, provider))
    }
}

fn require_admin(env: &Env) -> Result<Address, SignalError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&ADMIN_KEY)
        .ok_or(SignalError::NotInitialized)?;
    admin.require_auth();
    Ok(admin)
}

fn require_reviewer(env: &Env, reviewer: &Address) -> Result<(), SignalError> {
    if env.storage().persistent().has(&(REVIEWER_KEY, reviewer.clone())) {
        Ok(())
    } else {
        Err(SignalError::Unauthorized)
    }
}

fn store_revision(env: &Env, signal_id: u64, revision: u32, creator: Address, content: String) {
    let key = (SIGNAL_REVISION_KEY, signal_id, revision);
    let entry = SignalRevision { revision, creator, content, created_at: env.ledger().timestamp() };
    env.storage().persistent().set(&key, &entry);
}

fn load_case(env: &Env, case_id: u64) -> Result<ModerationCase, SignalError> {
    env.storage()
        .persistent()
        .get(&(CASE_KEY, case_id))
        .ok_or(SignalError::CaseNotFound)
}

fn load_pending_appeal(env: &Env, case_id: u64) -> Result<(ModerationCase, Appeal), SignalError> {
    let case = load_case(env, case_id)?;
    if case.status != CaseStatus::Appealed {
        return Err(SignalError::InvalidCaseState);
    }
    let appeal = env
        .storage()
        .persistent()
        .get(&(APPEAL_KEY, case_id))
        .ok_or(SignalError::CaseNotFound)?;
    Ok((case, appeal))
}

#[allow(clippy::too_many_arguments)]
fn finish_appeal(
    env: &Env,
    case_id: u64,
    mut case: ModerationCase,
    mut appeal: Appeal,
    approved: bool,
    final_reason: String,
    resolver: Option<Address>,
    timed_out: bool,
) {
    appeal.approved = Some(approved);
    appeal.final_reason = Some(final_reason.clone());
    env.storage().persistent().set(&(APPEAL_KEY, case_id), &appeal);
    case.status = CaseStatus::AppealResolved;
    env.storage().persistent().set(&(CASE_KEY, case_id), &case);
    env.events().publish(
        (symbol_short!("appeal"), symbol_short!("resolved"), case_id),
        AppealResolvedEvent {
            case_id,
            signal_id: case.signal_id,
            approved,
            final_reason,
            resolver,
            timed_out,
        },
    );
}

/// Number of whole decay intervals due for `record` at `now`.
fn due_intervals(record: &ReputationRecord, now: u64, schedule: &DecaySchedule) -> u64 {
    if schedule.decay_interval == 0 || now <= record.last_updated {
        return 0;
    }
    let elapsed = now - record.last_updated;
    if elapsed <= schedule.grace_period {
        return 0;
    }
    (elapsed - schedule.grace_period) / schedule.decay_interval
}

/// Score after `intervals` decay intervals; identical to `apply_decay` once
/// all due intervals are applied.
fn decayed_score(record: &ReputationRecord, intervals: u64, schedule: &DecaySchedule) -> i128 {
    if intervals == 0 {
        return record.score;
    }
    let lost = record
        .score
        .saturating_mul(schedule.decay_rate_bps as i128)
        .saturating_mul(intervals as i128)
        / 10_000;
    SignalRegistry::clamp(record.score.saturating_sub(lost), schedule)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::SignalRegistryClient;
    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};

    fn setup(env: &Env) -> (SignalRegistryClient<'_>, Address) {
        env.mock_all_auths();
        let id = env.register(SignalRegistry, ());
        let client = SignalRegistryClient::new(env, &id);
        let admin = Address::generate(env);
        let schedule = DecaySchedule {
            decay_rate_bps: 10,
            decay_interval: 100,
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 1_000_000,
        };
        client.initialize(&admin, &schedule);
        client.create_category(&1, &String::from_str(env, "forex"));
        (client, admin)
    }

    fn s(env: &Env, v: &str) -> String {
        String::from_str(env, v)
    }

    #[test]
    fn old_revisions_remain_unchanged_after_edits() {
        let env = Env::default();
        let (client, _) = setup(&env);
        let provider = Address::generate(&env);
        let id = client.create_signal(&provider, &1, &s(&env, "buy XLM"));
        let first = client.get_signal_revision(&id, &1).unwrap();
        env.ledger().with_mut(|l| l.timestamp += 10);
        assert_eq!(client.edit_signal(&provider, &id, &s(&env, "sell XLM")), 2);
        assert_eq!(client.edit_signal(&provider, &id, &s(&env, "hold XLM")), 3);

        assert_eq!(client.get_signal_revision(&id, &1).unwrap(), first);
        assert_eq!(client.get_signal(&id).unwrap().current_revision, 3);
        let page = client.get_signal_history(&id, &2, &1);
        assert_eq!(page.len(), 1);
        assert_eq!(page.get(0).unwrap().content, s(&env, "sell XLM"));
        assert_eq!(page.get(0).unwrap().creator, provider);
        assert_eq!(client.get_signal_history(&id, &1, &50).len(), 3);
        assert_eq!(
            client.try_get_signal_history(&id, &1, &(MAX_REVISION_PAGE + 1)),
            Err(Ok(SignalError::InvalidBatchSize))
        );
        let other = Address::generate(&env);
        assert_eq!(
            client.try_edit_signal(&other, &id, &s(&env, "spam")),
            Err(Ok(SignalError::Unauthorized))
        );
    }

    #[test]
    fn appeal_lifecycle_enforces_authorization_and_states() {
        let env = Env::default();
        let (client, _) = setup(&env);
        let provider = Address::generate(&env);
        let reviewer = Address::generate(&env);
        client.set_appeal_reviewer(&reviewer, &true);
        client.set_appeal_reviewer(&provider, &true);
        let signal = client.create_signal(&provider, &1, &s(&env, "buy XLM"));
        let case = client.open_moderation_case(&signal, &s(&env, "misleading"));

        // Unresolved case cannot be appealed.
        assert_eq!(
            client.try_submit_appeal(&provider, &case, &s(&env, "unfair")),
            Err(Ok(SignalError::InvalidCaseState))
        );
        client.decide_moderation_case(&case);
        client.submit_appeal(&provider, &case, &s(&env, "unfair"));

        // Appellant cannot review their own appeal even as a reviewer.
        assert_eq!(client.try_review_appeal(&provider, &case), Err(Ok(SignalError::SelfReview)));
        client.review_appeal(&reviewer, &case);
        client.resolve_appeal(&reviewer, &case, &true, &s(&env, "evidence accepted"));

        let event = env.events().all().last().unwrap();
        let payload: AppealResolvedEvent =
            soroban_sdk::TryFromVal::try_from_val(&env, &event.2).unwrap();
        assert_eq!(payload.final_reason, s(&env, "evidence accepted"));
        assert!(payload.approved);

        // Already resolved case cannot be appealed again.
        assert_eq!(
            client.try_submit_appeal(&provider, &case, &s(&env, "again")),
            Err(Ok(SignalError::InvalidCaseState))
        );
    }

    #[test]
    fn appeal_times_out() {
        let env = Env::default();
        let (client, _) = setup(&env);
        let provider = Address::generate(&env);
        let reviewer = Address::generate(&env);
        client.set_appeal_reviewer(&reviewer, &true);
        let signal = client.create_signal(&provider, &1, &s(&env, "buy XLM"));
        let case = client.open_moderation_case(&signal, &s(&env, "spam"));
        client.decide_moderation_case(&case);
        client.submit_appeal(&provider, &case, &s(&env, "unfair"));
        client.review_appeal(&reviewer, &case);

        assert_eq!(client.try_expire_appeal(&case), Err(Ok(SignalError::AppealNotExpired)));
        env.ledger().with_mut(|l| l.timestamp += APPEAL_TIMEOUT_SECONDS);
        assert_eq!(
            client.try_resolve_appeal(&reviewer, &case, &true, &s(&env, "late")),
            Err(Ok(SignalError::AppealExpired))
        );
        client.expire_appeal(&case);
        let appeal = client.get_appeal(&case).unwrap();
        assert_eq!(appeal.approved, Some(false));
        assert_eq!(client.get_moderation_case(&case).unwrap().status, CaseStatus::AppealResolved);
    }

    #[test]
    fn category_removal_and_reimport_are_tombstoned() {
        let env = Env::default();
        let (client, _) = setup(&env);
        let provider = Address::generate(&env);
        let signal = client.create_signal(&provider, &1, &s(&env, "buy XLM"));
        client.delete_category(&1);

        // Historical reads keep the label.
        assert_eq!(client.get_signal_category_label(&signal).unwrap(), s(&env, "forex"));
        assert!(client.get_category(&1).unwrap().tombstoned);
        // New signals cannot target the tombstone.
        assert_eq!(
            client.try_create_signal(&provider, &1, &s(&env, "new")),
            Err(Ok(SignalError::CategoryTombstoned))
        );
        // Re-import (with same or different semantics) is rejected.
        assert_eq!(
            client.try_create_category(&1, &s(&env, "crypto")),
            Err(Ok(SignalError::CategoryTombstoned))
        );
        assert_eq!(
            client.try_create_category(&1, &s(&env, "forex")),
            Err(Ok(SignalError::CategoryTombstoned))
        );
        client.create_category(&2, &s(&env, "crypto"));
        client.create_signal(&provider, &2, &s(&env, "new"));
    }

    #[test]
    fn decay_checkpoints_are_bounded_and_converge() {
        let env = Env::default();
        let (client, _) = setup(&env);
        let provider = Address::generate(&env);
        client.update_reputation(&provider, &1_000_000);
        // 500 intervals of inactivity.
        env.ledger().with_mut(|l| l.timestamp += 100 * 500);
        let expected = client.get_reputation(&provider);

        let mut calls = 0;
        loop {
            env.cost_estimate().budget().reset_default();
            let cp = client.advance_reputation_decay(&provider);
            assert!(env.cost_estimate().budget().cpu_instruction_cost() < 5_000_000);
            calls += 1;
            assert_eq!(cp.intervals_applied, (calls * MAX_DECAY_INTERVALS_PER_CALL).min(500));
            if cp.complete {
                assert_eq!(cp.score, expected);
                break;
            }
        }
        assert_eq!(calls, 500u64.div_ceil(MAX_DECAY_INTERVALS_PER_CALL));
        // Further calls are idempotent.
        assert_eq!(client.advance_reputation_decay(&provider).score, expected);
        assert_eq!(client.get_decay_checkpoint(&provider).unwrap().intervals_applied, 500);
    }
}
