//! Issue #506: Provider reputation system with community voting.
//!
//! Voting power = stake_amount / VOTE_POWER_DIVISOR (1 vote per 10 XLM staked).
//! Reputation score is adjusted after each vote window closes.
//! Dispute resolution: if downvotes exceed DISPUTE_THRESHOLD_BPS of total votes,
//! a dispute is opened and the provider's score is frozen until resolved.
//!
//! Issue #539: Disputed providers can appeal an open dispute. An appeal must
//! be decided by an admin within APPEAL_WINDOW_SECS; if it isn't,
//! `process_appeal_timeout` auto-advances the appeal to `Rejected` so a
//! dispute can't be left in limbo forever by an unresponsive admin.

use soroban_sdk::{contracterror, contracttype, Address, Env, Map, Symbol, Vec};

/// 10 XLM in stroops = 1 vote unit
pub const VOTE_POWER_DIVISOR: i128 = 100_000_000;
/// Voting window: 7 days in seconds
pub const VOTE_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;
/// Dispute threshold: if downvotes >= 30% of total votes, open dispute
pub const DISPUTE_THRESHOLD_BPS: u32 = 3_000;
/// Max reputation score
pub const MAX_REPUTATION: u32 = 100;
/// Score adjustment per vote window: ±5 points
pub const SCORE_DELTA: u32 = 5;
/// Minimum score floor (recovery cannot go below this)
pub const MIN_REPUTATION: u32 = 0;
/// Window an admin has to decide a submitted appeal before it auto-rejects: 3 days
pub const APPEAL_WINDOW_SECS: u64 = 3 * 24 * 60 * 60;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoteKind {
    Up,
    Down,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct VoteRecord {
    pub voter: Address,
    pub kind: VoteKind,
    pub power: u32,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisputeStatus {
    Open,
    Resolved,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppealStatus {
    /// No appeal has been submitted for this dispute.
    None,
    /// An appeal was submitted and is awaiting an admin decision.
    Pending,
    /// An admin approved the appeal; the dispute is resolved in the provider's favor.
    Approved,
    /// The appeal was rejected by an admin, or auto-rejected after timing out.
    Rejected,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct DisputeRecord {
    pub provider: Address,
    pub opened_at: u64,
    pub status: DisputeStatus,
    pub downvote_bps: u32,
    pub appeal_status: AppealStatus,
    /// Ledger timestamp the current appeal was submitted at; 0 if none was submitted.
    pub appeal_submitted_at: u64,
}

/// Issue #782: renumbered from 1-6, which collided with `AdminError`
/// (also 1-36) — two different failure conditions decoded to the same
/// contract-wide `u32` error code.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DisputeError {
    Unauthorized = 1600,
    DisputeNotFound = 1601,
    DisputeNotOpen = 1602,
    AppealAlreadySubmitted = 1603,
    AppealNotPending = 1604,
    AppealWindowNotElapsed = 1605,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ReputationHistory {
    /// (timestamp, score) pairs — last 10 entries
    pub entries: Vec<(u64, u32)>,
}

#[contracttype]
#[derive(Clone)]
pub enum VotingKey {
    /// provider -> Map<voter_address, VoteRecord> for current window
    CurrentVotes(Address),
    /// provider -> window_start timestamp
    VoteWindowStart(Address),
    /// provider -> DisputeRecord
    Dispute(Address),
    /// provider -> ReputationHistory
    History(Address),
    /// provider -> bool (score frozen due to open dispute)
    ScoreFrozen(Address),
}

/// Cast a vote for a provider. One vote per voter per window; re-voting replaces prior vote.
/// Voting power is derived from the voter's stake.
pub fn cast_vote(env: &Env, voter: Address, provider: Address, kind: VoteKind, voter_stake: i128) {
    let now = env.ledger().timestamp();
    let power = (voter_stake / VOTE_POWER_DIVISOR)
        .max(0)
        .min(u32::MAX as i128) as u32;

    // Rotate window if expired
    let window_key = VotingKey::VoteWindowStart(provider.clone());
    let mut window_start: u64 = env.storage().persistent().get(&window_key).unwrap_or(0);
    if window_start == 0 {
        window_start = now;
        env.storage().persistent().set(&window_key, &window_start);
    }
    let votes_key = VotingKey::CurrentVotes(provider.clone());

    let mut votes: Map<Address, VoteRecord> = if now >= window_start + VOTE_WINDOW_SECS {
        // New window: tally old window first, then reset
        let old_votes: Map<Address, VoteRecord> = env
            .storage()
            .persistent()
            .get(&votes_key)
            .unwrap_or(Map::new(env));
        tally_and_adjust(env, &provider, &old_votes);
        env.storage().persistent().set(&window_key, &now);
        Map::new(env)
    } else {
        env.storage()
            .persistent()
            .get(&votes_key)
            .unwrap_or(Map::new(env))
    };

    votes.set(
        voter.clone(),
        VoteRecord {
            voter,
            kind,
            power,
            timestamp: now,
        },
    );
    env.storage().persistent().set(&votes_key, &votes);
}

/// Tally votes and adjust reputation score. Called at window close.
fn tally_and_adjust(env: &Env, provider: &Address, votes: &Map<Address, VoteRecord>) {
    if votes.is_empty() {
        return;
    }

    // Score is frozen during open dispute
    let frozen_key = VotingKey::ScoreFrozen(provider.clone());
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&frozen_key)
        .unwrap_or(false)
    {
        return;
    }

    let mut up_power: u64 = 0;
    let mut down_power: u64 = 0;
    for key in votes.keys() {
        if let Some(record) = votes.get(key) {
            match record.kind {
                VoteKind::Up => up_power += record.power as u64,
                VoteKind::Down => down_power += record.power as u64,
            }
        }
    }

    let rep_key = crate::StorageKey::ProviderReputationScore(provider.clone());
    let current: u32 = env.storage().instance().get(&rep_key).unwrap_or(50);

    let new_score = if up_power >= down_power {
        (current + SCORE_DELTA).min(MAX_REPUTATION)
    } else {
        current.saturating_sub(SCORE_DELTA).max(MIN_REPUTATION)
    };

    env.storage().instance().set(&rep_key, &new_score);
    append_history(env, provider, new_score);

    env.events().publish(
        (
            Symbol::new(env, "reputation_vote_tallied"),
            provider.clone(),
        ),
        (up_power, down_power, new_score),
    );

    check_dispute_threshold(env, provider, votes);
}

/// Check if downvotes exceed dispute threshold; open dispute if so.
fn check_dispute_threshold(env: &Env, provider: &Address, votes: &Map<Address, VoteRecord>) {
    let mut total: u64 = 0;
    let mut down: u64 = 0;
    for key in votes.keys() {
        if let Some(record) = votes.get(key) {
            total += record.power as u64;
            if record.kind == VoteKind::Down {
                down += record.power as u64;
            }
        }
    }
    if total == 0 {
        return;
    }
    let down_bps = ((down * 10_000) / total) as u32;
    if down_bps >= DISPUTE_THRESHOLD_BPS {
        open_dispute(env, provider, down_bps);
    }
}

/// Open a dispute and freeze the provider's reputation score.
fn open_dispute(env: &Env, provider: &Address, downvote_bps: u32) {
    let dispute_key = VotingKey::Dispute(provider.clone());
    // Don't re-open if already open
    if let Some(existing) = env
        .storage()
        .persistent()
        .get::<_, DisputeRecord>(&dispute_key)
    {
        if existing.status == DisputeStatus::Open {
            return;
        }
    }

    let record = DisputeRecord {
        provider: provider.clone(),
        opened_at: env.ledger().timestamp(),
        status: DisputeStatus::Open,
        downvote_bps,
        appeal_status: AppealStatus::None,
        appeal_submitted_at: 0,
    };
    env.storage().persistent().set(&dispute_key, &record);
    env.storage()
        .persistent()
        .set(&VotingKey::ScoreFrozen(provider.clone()), &true);

    env.events().publish(
        (Symbol::new(env, "dispute_opened"), provider.clone()),
        downvote_bps,
    );
}

/// Admin resolves a dispute directly. If `restore` is true, score is unfrozen and
/// recovery begins. Any appeal still `Pending` is cleared, since the dispute it was
/// raised against is being closed out by this call.
pub fn resolve_dispute(env: &Env, provider: Address, restore: bool) {
    let dispute_key = VotingKey::Dispute(provider.clone());
    let mut record: DisputeRecord =
        env.storage()
            .persistent()
            .get(&dispute_key)
            .unwrap_or(DisputeRecord {
                provider: provider.clone(),
                opened_at: 0,
                status: DisputeStatus::Resolved,
                downvote_bps: 0,
                appeal_status: AppealStatus::None,
                appeal_submitted_at: 0,
            });

    record.status = DisputeStatus::Resolved;
    if record.appeal_status == AppealStatus::Pending {
        record.appeal_status = AppealStatus::None;
    }
    env.storage().persistent().set(&dispute_key, &record);
    env.storage()
        .persistent()
        .remove(&VotingKey::ScoreFrozen(provider.clone()));

    if restore {
        // Apply one recovery step immediately
        apply_recovery(env, &provider);
    }

    env.events().publish(
        (Symbol::new(env, "dispute_resolved"), provider.clone()),
        restore,
    );
}

/// Disputed provider submits an appeal of an open dispute. Only the provider the
/// dispute was opened against may appeal it, and only once per dispute.
pub fn submit_appeal(env: &Env, provider: Address) -> Result<(), DisputeError> {
    provider.require_auth();

    let dispute_key = VotingKey::Dispute(provider.clone());
    let mut record: DisputeRecord = env
        .storage()
        .persistent()
        .get(&dispute_key)
        .ok_or(DisputeError::DisputeNotFound)?;

    if record.status != DisputeStatus::Open {
        return Err(DisputeError::DisputeNotOpen);
    }
    if record.appeal_status != AppealStatus::None {
        return Err(DisputeError::AppealAlreadySubmitted);
    }

    record.appeal_status = AppealStatus::Pending;
    record.appeal_submitted_at = env.ledger().timestamp();
    env.storage().persistent().set(&dispute_key, &record);

    env.events().publish(
        (Symbol::new(env, "appeal_submitted"), provider.clone()),
        record.appeal_submitted_at,
    );
    Ok(())
}

/// Admin decides a pending appeal. Approving resolves the dispute in the provider's
/// favor (score unfrozen and one recovery step applied); rejecting leaves the original
/// dispute in place for the admin to resolve separately via `resolve_dispute`.
pub fn resolve_appeal(env: &Env, provider: Address, approve: bool) -> Result<(), DisputeError> {
    let dispute_key = VotingKey::Dispute(provider.clone());
    let mut record: DisputeRecord = env
        .storage()
        .persistent()
        .get(&dispute_key)
        .ok_or(DisputeError::DisputeNotFound)?;

    if record.appeal_status != AppealStatus::Pending {
        return Err(DisputeError::AppealNotPending);
    }

    record.appeal_status = if approve {
        AppealStatus::Approved
    } else {
        AppealStatus::Rejected
    };
    env.storage().persistent().set(&dispute_key, &record);

    if approve {
        resolve_dispute(env, provider.clone(), true);
    }

    env.events().publish(
        (Symbol::new(env, "appeal_resolved"), provider.clone()),
        approve,
    );
    Ok(())
}

/// Auto-advances a pending appeal to `Rejected` once it has sat unresolved for more
/// than [`APPEAL_WINDOW_SECS`]. Callable by anyone — it only enforces a deterministic
/// timeout, so it doesn't need to be gated to a specific caller. This prevents a
/// dispute from being left in limbo indefinitely because an admin never decided the
/// appeal; the underlying dispute is unaffected and can still be resolved normally.
pub fn process_appeal_timeout(env: &Env, provider: Address) -> Result<(), DisputeError> {
    let dispute_key = VotingKey::Dispute(provider.clone());
    let mut record: DisputeRecord = env
        .storage()
        .persistent()
        .get(&dispute_key)
        .ok_or(DisputeError::DisputeNotFound)?;

    if record.appeal_status != AppealStatus::Pending {
        return Err(DisputeError::AppealNotPending);
    }
    if env.ledger().timestamp()
        < record
            .appeal_submitted_at
            .saturating_add(APPEAL_WINDOW_SECS)
    {
        return Err(DisputeError::AppealWindowNotElapsed);
    }

    record.appeal_status = AppealStatus::Rejected;
    env.storage().persistent().set(&dispute_key, &record);

    env.events().publish(
        (Symbol::new(env, "appeal_timed_out"), provider.clone()),
        record.appeal_submitted_at,
    );
    Ok(())
}

/// Recovery: increment score by SCORE_DELTA (called after dispute resolution or manually).
pub fn apply_recovery(env: &Env, provider: &Address) {
    let rep_key = crate::StorageKey::ProviderReputationScore(provider.clone());
    let current: u32 = env.storage().instance().get(&rep_key).unwrap_or(0);
    let new_score = (current + SCORE_DELTA).min(MAX_REPUTATION);
    env.storage().instance().set(&rep_key, &new_score);
    append_history(env, provider, new_score);
}

/// Append a score entry to the provider's reputation history (capped at 10 entries).
fn append_history(env: &Env, provider: &Address, score: u32) {
    let hist_key = VotingKey::History(provider.clone());
    let mut history: ReputationHistory =
        env.storage()
            .persistent()
            .get(&hist_key)
            .unwrap_or(ReputationHistory {
                entries: Vec::new(env),
            });

    let now = env.ledger().timestamp();
    history.entries.push_back((now, score));

    // Keep only last 10
    while history.entries.len() > 10 {
        history.entries.remove(0);
    }

    env.storage().persistent().set(&hist_key, &history);
}

/// Get reputation history for a provider.
pub fn get_reputation_history(env: &Env, provider: &Address) -> ReputationHistory {
    env.storage()
        .persistent()
        .get(&VotingKey::History(provider.clone()))
        .unwrap_or(ReputationHistory {
            entries: Vec::new(env),
        })
}

/// Get current dispute record for a provider.
pub fn get_dispute(env: &Env, provider: &Address) -> Option<DisputeRecord> {
    env.storage()
        .persistent()
        .get(&VotingKey::Dispute(provider.clone()))
}

/// Get current votes for a provider in the active window.
pub fn get_current_votes(env: &Env, provider: &Address) -> Map<Address, VoteRecord> {
    env.storage()
        .persistent()
        .get(&VotingKey::CurrentVotes(provider.clone()))
        .unwrap_or(Map::new(env))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SignalRegistry;
    use crate::SignalRegistryClient;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    fn with_registry<R>(f: impl FnOnce(&Env, &Address, &SignalRegistryClient, &Address) -> R) -> R {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        #[allow(deprecated)]
        let cid = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(&env, &cid);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        f(&env, &cid, &client, &admin)
    }

    fn cast_in_contract(
        env: &Env,
        cid: &Address,
        voter: Address,
        provider: Address,
        kind: VoteKind,
        stake: i128,
    ) {
        env.as_contract(cid, || cast_vote(env, voter, provider, kind, stake));
    }

    fn in_contract<R>(env: &Env, cid: &Address, f: impl FnOnce(&Env) -> R) -> R {
        env.as_contract(cid, || f(env))
    }

    #[test]
    fn test_upvote_increases_score() {
        with_registry(|env, cid, client, _admin| {
            let voter = Address::generate(env);
            let provider = Address::generate(env);
            in_contract(env, cid, |env| {
                env.storage().instance().set(
                    &crate::StorageKey::ProviderReputationScore(provider.clone()),
                    &50u32,
                );
            });

            cast_in_contract(
                env,
                cid,
                voter.clone(),
                provider.clone(),
                VoteKind::Up,
                1_000_000_000,
            );

            env.ledger().set_timestamp(1_000_000 + VOTE_WINDOW_SECS + 1);

            let voter2 = Address::generate(env);
            cast_in_contract(
                env,
                cid,
                voter2,
                provider.clone(),
                VoteKind::Up,
                1_000_000_000,
            );

            let score = client.get_provider_reputation_score(&provider);
            assert_eq!(score, 55);
        });
    }

    #[test]
    fn test_downvote_decreases_score() {
        with_registry(|env, cid, client, _admin| {
            let voter = Address::generate(env);
            let provider = Address::generate(env);
            in_contract(env, cid, |env| {
                env.storage().instance().set(
                    &crate::StorageKey::ProviderReputationScore(provider.clone()),
                    &50u32,
                );
            });

            cast_in_contract(
                env,
                cid,
                voter,
                provider.clone(),
                VoteKind::Down,
                1_000_000_000,
            );
            env.ledger().set_timestamp(1_000_000 + VOTE_WINDOW_SECS + 1);
            let voter2 = Address::generate(env);
            cast_in_contract(env, cid, voter2, provider.clone(), VoteKind::Up, 0);

            let score = client.get_provider_reputation_score(&provider);
            assert_eq!(score, 45);
        });
    }

    #[test]
    fn test_dispute_opens_when_threshold_exceeded() {
        with_registry(|env, cid, _client, _admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            let dispute = in_contract(env, cid, |env| get_dispute(env, &provider));
            assert!(dispute.is_some());
            assert_eq!(dispute.unwrap().status, DisputeStatus::Open);
        });
    }

    #[test]
    fn test_resolve_dispute_unfreezes_score() {
        with_registry(|env, cid, _client, _admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            in_contract(env, cid, |env| {
                resolve_dispute(env, provider.clone(), true);
            });

            in_contract(env, cid, |env| {
                let dispute = get_dispute(env, &provider).unwrap();
                assert_eq!(dispute.status, DisputeStatus::Resolved);
                let frozen: bool = env
                    .storage()
                    .persistent()
                    .get(&VotingKey::ScoreFrozen(provider.clone()))
                    .unwrap_or(false);
                assert!(!frozen);
            });
        });
    }

    #[test]
    fn test_reputation_history_tracked() {
        with_registry(|env, cid, _client, _admin| {
            let provider = Address::generate(env);
            in_contract(env, cid, |env| {
                env.storage().instance().set(
                    &crate::StorageKey::ProviderReputationScore(provider.clone()),
                    &50u32,
                );
                apply_recovery(env, &provider);
                apply_recovery(env, &provider);

                let history = get_reputation_history(env, &provider);
                assert_eq!(history.entries.len(), 2);
                assert_eq!(history.entries.get(0).unwrap().1, 55);
                assert_eq!(history.entries.get(1).unwrap().1, 60);
            });
        });
    }

    #[test]
    fn test_history_capped_at_10() {
        with_registry(|env, cid, _client, _admin| {
            let provider = Address::generate(env);
            in_contract(env, cid, |env| {
                env.storage().instance().set(
                    &crate::StorageKey::ProviderReputationScore(provider.clone()),
                    &0u32,
                );
                for _ in 0..15u32 {
                    apply_recovery(env, &provider);
                }
                let history = get_reputation_history(env, &provider);
                assert_eq!(history.entries.len(), 10);
            });
        });
    }

    fn open_dispute_for(env: &Env, cid: &Address, provider: &Address) {
        for _ in 0..4u32 {
            let voter = Address::generate(env);
            cast_in_contract(
                env,
                cid,
                voter,
                provider.clone(),
                VoteKind::Down,
                1_000_000_000,
            );
        }
        let voter = Address::generate(env);
        cast_in_contract(
            env,
            cid,
            voter,
            provider.clone(),
            VoteKind::Up,
            1_000_000_000,
        );
        env.ledger().set_timestamp(1_000_000 + VOTE_WINDOW_SECS + 1);
        let voter2 = Address::generate(env);
        cast_in_contract(env, cid, voter2, provider.clone(), VoteKind::Up, 0);
    }

    #[test]
    fn test_submit_appeal_marks_pending() {
        with_registry(|env, cid, client, _admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            client.submit_dispute_appeal(&provider);

            in_contract(env, cid, |env| {
                let dispute = get_dispute(env, &provider).unwrap();
                assert_eq!(dispute.appeal_status, AppealStatus::Pending);
                assert_eq!(dispute.appeal_submitted_at, env.ledger().timestamp());
            });
        });
    }

    #[test]
    fn test_submit_appeal_without_dispute_fails() {
        with_registry(|env, _cid, client, _admin| {
            let provider = Address::generate(env);
            let result = client.try_submit_dispute_appeal(&provider);
            assert_eq!(result, Err(Ok(DisputeError::DisputeNotFound)));
        });
    }

    #[test]
    fn test_submit_appeal_twice_fails() {
        with_registry(|env, cid, client, _admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            client.submit_dispute_appeal(&provider);
            let result = client.try_submit_dispute_appeal(&provider);
            assert_eq!(result, Err(Ok(DisputeError::AppealAlreadySubmitted)));
        });
    }

    #[test]
    fn test_resolve_appeal_approve_resolves_dispute_and_unfreezes_score() {
        with_registry(|env, cid, client, admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            client.submit_dispute_appeal(&provider);
            client.resolve_dispute_appeal(admin, &provider, &true);

            in_contract(env, cid, |env| {
                let dispute = get_dispute(env, &provider).unwrap();
                assert_eq!(dispute.appeal_status, AppealStatus::Approved);
                assert_eq!(dispute.status, DisputeStatus::Resolved);
                let frozen: bool = env
                    .storage()
                    .persistent()
                    .get(&VotingKey::ScoreFrozen(provider.clone()))
                    .unwrap_or(false);
                assert!(!frozen);
            });
        });
    }

    #[test]
    fn test_resolve_appeal_reject_leaves_dispute_open() {
        with_registry(|env, cid, client, admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            client.submit_dispute_appeal(&provider);
            client.resolve_dispute_appeal(admin, &provider, &false);

            in_contract(env, cid, |env| {
                let dispute = get_dispute(env, &provider).unwrap();
                assert_eq!(dispute.appeal_status, AppealStatus::Rejected);
                assert_eq!(dispute.status, DisputeStatus::Open);

                resolve_dispute(env, provider.clone(), true);
                assert_eq!(
                    get_dispute(env, &provider).unwrap().status,
                    DisputeStatus::Resolved
                );
            });
        });
    }

    #[test]
    fn test_resolve_appeal_without_pending_appeal_fails() {
        with_registry(|env, cid, client, admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            let result = client.try_resolve_dispute_appeal(admin, &provider, &true);
            assert_eq!(result, Err(Ok(DisputeError::AppealNotPending)));
        });
    }

    #[test]
    fn test_process_appeal_timeout_before_window_elapsed_fails() {
        with_registry(|env, cid, client, _admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            client.submit_dispute_appeal(&provider);

            env.ledger()
                .set_timestamp(env.ledger().timestamp() + APPEAL_WINDOW_SECS - 1);
            let result = client.try_process_dispute_appeal_timeout(&provider);
            assert_eq!(result, Err(Ok(DisputeError::AppealWindowNotElapsed)));
        });
    }

    #[test]
    fn test_process_appeal_timeout_after_window_auto_rejects() {
        with_registry(|env, cid, client, _admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            client.submit_dispute_appeal(&provider);

            env.ledger()
                .set_timestamp(env.ledger().timestamp() + APPEAL_WINDOW_SECS + 1);
            client.process_dispute_appeal_timeout(&provider);

            in_contract(env, cid, |env| {
                let dispute = get_dispute(env, &provider).unwrap();
                assert_eq!(dispute.appeal_status, AppealStatus::Rejected);
                assert_eq!(dispute.status, DisputeStatus::Open);
            });
        });
    }

    #[test]
    fn test_process_appeal_timeout_without_pending_appeal_fails() {
        with_registry(|env, cid, client, _admin| {
            let provider = Address::generate(env);
            open_dispute_for(env, cid, &provider);

            let result = client.try_process_dispute_appeal_timeout(&provider);
            assert_eq!(result, Err(Ok(DisputeError::AppealNotPending)));
        });
    }
}
