use core::cmp::min;

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, String, Vec};

use crate::proposals::{self, ProposalStatus, VoteType};
use crate::{checked_mul, GovernanceError, StorageKey};

const PRECISION: i128 = 10_000;
const MAX_REPUTATION: u32 = 10_000;
pub const MAX_REPUTATION_USERS: u32 = 100;

// ── Decay Schedule Tiers ─────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReputationTier {
    Bronze,
    Silver,
    Gold,
    Platinum,
}

impl ReputationTier {
    /// Decay rate in basis points (1 BPS = 0.01%) applied per inactive day.
    pub fn decay_rate_bps(&self) -> u32 {
        match self {
            ReputationTier::Bronze => 50,
            ReputationTier::Silver => 30,
            ReputationTier::Gold => 20,
            ReputationTier::Platinum => 10,
        }
    }

    /// Minimum inactive days before this tier's decay begins.
    pub fn grace_period_days(&self) -> u64 {
        match self {
            ReputationTier::Bronze => 30,
            ReputationTier::Silver => 60,
            ReputationTier::Gold => 90,
            ReputationTier::Platinum => 180,
        }
    }

    /// Maximum days of decay to apply (cap to prevent total wipeout).
    pub fn max_decay_days(&self) -> u64 {
        match self {
            ReputationTier::Bronze => 90,
            ReputationTier::Silver => 180,
            ReputationTier::Gold => 270,
            ReputationTier::Platinum => 365,
        }
    }
}

// ── Staleness Levels ─────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StalenessLevel {
    /// No manual override; staleness is derived from last activity.
    Auto,
    Active,
    Aging,
    Stale,
    Critical,
}

// ── Reputation Configuration ─────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationConfig {
    pub decay_enabled: bool,
    pub stale_penalty_enabled: bool,
    pub default_tier: ReputationTier,
}

impl Default for ReputationConfig {
    fn default() -> Self {
        ReputationConfig {
            decay_enabled: true,
            stale_penalty_enabled: true,
            default_tier: ReputationTier::Bronze,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipationHistory {
    pub proposals_created: u32,
    pub proposals_succeeded: u32,
    pub votes_cast: u32,
    pub votes_aligned_with_outcome: u32,
    pub committee_memberships: u32,
    pub committee_decisions_approved: u32,
    pub delegations_received: u32,
    pub total_tokens_delegated: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Badge {
    ActiveVoter(u32),
    ProposalAuthor(u32),
    CommitteeMember(String),
    EarlyAdopter,
    TopDelegator,
    ConsistentParticipant(u32),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovernanceReputation {
    pub user: Address,
    pub reputation_score: u32,
    pub participation_history: ParticipationHistory,
    pub badges: Vec<Badge>,
    pub last_activity: u64,
    pub decay_rate: u32,
    /// Current tier for decay schedule (Bronze/Silver/Gold/Platinum).
    pub tier: ReputationTier,
    /// Override grace period in days (0 = use tier default).
    pub grace_override: u64,
    /// Manual staleness level override (`Auto` = auto-detected from activity).
    pub staleness_override: StalenessLevel,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationState {
    pub reputations: Map<Address, GovernanceReputation>,
    pub users: Vec<Address>,
}

// ── Storage helpers for ReputationConfig ──────────────────────────────────

pub fn get_reputation_config(env: &Env) -> ReputationConfig {
    env.storage()
        .instance()
        .get(&StorageKey::ReputationConfig)
        .unwrap_or_else(|| ReputationConfig::default())
}

pub fn put_reputation_config(env: &Env, config: &ReputationConfig) {
    env.storage()
        .instance()
        .set(&StorageKey::ReputationConfig, config);
}

// ── Staleness detection ──────────────────────────────────────────────────

/// Detect the staleness level of a user based on their last activity timestamp.
pub fn detect_staleness(env: &Env, last_activity: u64) -> StalenessLevel {
    let now = env.ledger().timestamp();
    if now <= last_activity {
        return StalenessLevel::Active;
    }
    let inactive_days = (now - last_activity) / 86_400;
    if inactive_days <= 30 {
        StalenessLevel::Active
    } else if inactive_days <= 90 {
        StalenessLevel::Aging
    } else if inactive_days <= 180 {
        StalenessLevel::Stale
    } else {
        StalenessLevel::Critical
    }
}

pub fn resolve_staleness(env: &Env, rep: &GovernanceReputation) -> StalenessLevel {
    match rep.staleness_override {
        StalenessLevel::Auto => detect_staleness(env, rep.last_activity),
        level => level,
    }
}

/// Calculate the effective staleness penalty multiplier (BPS basis).
/// 10_000 = no penalty; lower = reduced voting weight.
pub fn staleness_penalty_multiplier(level: &StalenessLevel) -> u32 {
    match level {
        StalenessLevel::Auto | StalenessLevel::Active => 10_000,
        StalenessLevel::Aging => 8_000,
        StalenessLevel::Stale => 5_000,
        StalenessLevel::Critical => 2_000,
    }
}

/// Compute the tier for a user based on participation history.
pub fn compute_tier(rep: &GovernanceReputation) -> ReputationTier {
    let p = &rep.participation_history;
    let total_actions =
        p.proposals_created + p.votes_cast + p.committee_memberships + p.delegations_received;

    if total_actions >= 500 {
        ReputationTier::Platinum
    } else if total_actions >= 200 {
        ReputationTier::Gold
    } else if total_actions >= 50 {
        ReputationTier::Silver
    } else {
        ReputationTier::Bronze
    }
}

pub fn empty_reputation_state(env: &Env) -> ReputationState {
    ReputationState {
        reputations: Map::new(env),
        users: Vec::new(env),
    }
}

pub fn get_reputation_state(env: &Env) -> ReputationState {
    env.storage()
        .instance()
        .get(&StorageKey::ReputationState)
        .unwrap_or_else(|| empty_reputation_state(env))
}

pub fn put_reputation_state(env: &Env, state: &ReputationState) {
    env.storage()
        .instance()
        .set(&StorageKey::ReputationState, state);
}

pub fn get_governance_reputation(env: &Env, user: Address) -> GovernanceReputation {
    let state = get_reputation_state(env);
    state.reputations.get(user.clone()).unwrap_or_else(|| {
        let rep = GovernanceReputation {
            user: user.clone(),
            reputation_score: 0,
            participation_history: ParticipationHistory {
                proposals_created: 0,
                proposals_succeeded: 0,
                votes_cast: 0,
                votes_aligned_with_outcome: 0,
                committee_memberships: 0,
                committee_decisions_approved: 0,
                delegations_received: 0,
                total_tokens_delegated: 0,
            },
            badges: Vec::new(env),
            last_activity: env.ledger().timestamp(),
            decay_rate: 10,
            tier: get_reputation_config(env).default_tier.clone(),
            grace_override: 0,
            staleness_override: StalenessLevel::Auto,
        };
        rep
    })
}

pub fn calculate_reputation_score(env: &Env, user: Address) -> Result<u32, GovernanceError> {
    let rep = get_governance_reputation(env, user);
    let mut score = 0u32;

    let proposal_score = min(
        2000,
        rep.participation_history
            .proposals_created
            .saturating_mul(100),
    );
    score = score.saturating_add(proposal_score);

    let success_rate = rep
        .participation_history
        .proposals_succeeded
        .saturating_mul(10_000)
        .checked_div(rep.participation_history.proposals_created)
        .unwrap_or(0);
    score = score.saturating_add(min(2000, success_rate.saturating_mul(2) / 10));

    let voting_score = min(
        2000,
        rep.participation_history.votes_cast.saturating_mul(20),
    );
    score = score.saturating_add(voting_score);

    let accuracy = rep
        .participation_history
        .votes_aligned_with_outcome
        .saturating_mul(10_000)
        .checked_div(rep.participation_history.votes_cast)
        .unwrap_or(0);
    score = score.saturating_add(min(2000, accuracy.saturating_mul(2) / 10));

    let committee_score = min(
        1000,
        rep.participation_history
            .committee_memberships
            .saturating_mul(200),
    );
    score = score.saturating_add(committee_score);

    let delegation_score = min(
        1000,
        rep.participation_history
            .delegations_received
            .saturating_mul(50),
    );
    score = score.saturating_add(delegation_score);

    score = score.saturating_add(calculate_badge_bonus(&rep.badges));

    // Apply tier-based decay schedule
    let config = get_reputation_config(env);
    let effective_tier = rep.tier.clone();
    score = apply_tiered_decay(
        env,
        score,
        rep.last_activity,
        rep.grace_override,
        &effective_tier,
    );

    // Apply staleness penalty if enabled
    if config.stale_penalty_enabled {
        let staleness = resolve_staleness(env, &rep);
        let penalty_mult = staleness_penalty_multiplier(&staleness);
        score = ((score as u64).saturating_mul(penalty_mult as u64) / 10_000) as u32;
    }

    Ok(min(MAX_REPUTATION, score))
}

pub fn record_proposal_creation(env: &Env, user: Address) -> Result<(), GovernanceError> {
    let mut state = get_reputation_state(env);
    let mut rep = get_governance_reputation(env, user.clone());

    rep.participation_history.proposals_created = rep
        .participation_history
        .proposals_created
        .saturating_add(1);
    rep.last_activity = env.ledger().timestamp();
    rep.tier = compute_tier(&rep);
    rep.reputation_score = calculate_reputation_score(env, user.clone())?;
    check_and_award_badges(env, &mut rep);

    upsert_rep(&mut state, &user, rep);
    put_reputation_state(env, &state);
    Ok(())
}

pub fn record_vote(
    env: &Env,
    user: Address,
    proposal_id: u64,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    let mut state = get_reputation_state(env);
    let mut rep = get_governance_reputation(env, user.clone());

    rep.participation_history.votes_cast = rep.participation_history.votes_cast.saturating_add(1);
    rep.last_activity = env.ledger().timestamp();
    rep.tier = compute_tier(&rep);
    rep.reputation_score = calculate_reputation_score(env, user.clone())?;
    check_and_award_badges(env, &mut rep);

    upsert_rep(&mut state, &user, rep);
    put_reputation_state(env, &state);

    let mut vote_records: Map<(Address, u64), VoteType> = env
        .storage()
        .instance()
        .get(&StorageKey::VoteRecords)
        .unwrap_or(Map::new(env));
    vote_records.set((user, proposal_id), vote_type);
    env.storage()
        .instance()
        .set(&StorageKey::VoteRecords, &vote_records);

    Ok(())
}

pub fn record_proposal_outcome(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let proposal = proposals::get_proposal(env, proposal_id)?;
    let outcome =
        proposal.status == ProposalStatus::Succeeded || proposal.status == ProposalStatus::Executed;

    let mut state = get_reputation_state(env);

    if outcome {
        let mut proposer_rep = get_governance_reputation(env, proposal.proposer.clone());
        proposer_rep.participation_history.proposals_succeeded = proposer_rep
            .participation_history
            .proposals_succeeded
            .saturating_add(1);
        proposer_rep.reputation_score = calculate_reputation_score(env, proposal.proposer.clone())?;
        upsert_rep(&mut state, &proposal.proposer, proposer_rep);
    }

    let mut idx = 0;
    while idx < proposal.voter_list.len() {
        let voter = proposal.voter_list.get(idx).unwrap();
        if let Some(vote) = proposal.voters.get(voter.clone()) {
            let aligned = matches!(
                (vote.vote_type, outcome),
                (VoteType::For, true) | (VoteType::Against, false)
            );
            if aligned {
                let mut rep = get_governance_reputation(env, voter.clone());
                rep.participation_history.votes_aligned_with_outcome = rep
                    .participation_history
                    .votes_aligned_with_outcome
                    .saturating_add(1);
                rep.reputation_score = calculate_reputation_score(env, voter.clone())?;
                upsert_rep(&mut state, &voter, rep);
            }
        }
        idx += 1;
    }

    put_reputation_state(env, &state);
    Ok(())
}

pub fn cast_reputation_weighted_vote(
    env: &Env,
    proposal_id: u64,
    voter: Address,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    voter.require_auth();

    let mut proposal = proposals::get_proposal(env, proposal_id)?;
    let now = env.ledger().timestamp();

    // Mirror the eligibility/timing gates enforced by `proposals::cast_vote`
    // (Issue #998) — this entrypoint shares the same `proposal.voters` tally,
    // so it must be just as resistant to duplicate participation, late votes,
    // and votes on proposals that are no longer open.
    if proposal.discussion_ends_at > 0 && now < proposal.discussion_ends_at {
        return Err(GovernanceError::DiscussionPeriodActive);
    }
    if now < proposal.voting_starts {
        return Err(GovernanceError::VotingNotStarted);
    }
    if now >= proposal.voting_ends {
        return Err(GovernanceError::VotingEnded);
    }
    if proposal.status != ProposalStatus::Pending && proposal.status != ProposalStatus::Active {
        return Err(GovernanceError::ProposalNotActive);
    }
    if proposal.voters.contains_key(voter.clone()) {
        return Err(GovernanceError::AlreadyVoted);
    }

    // Use the voting power snapshotted at proposal creation, not the voter's
    // *current* effective power. Reading live power here would let a voter
    // inflate their weight by delegating to themselves (or undelegating)
    // after the proposal was created, retroactively bypassing the
    // eligibility snapshot the token-weighted path already enforces.
    let token_power = crate::get_vote_snapshot(env, proposal_id, &voter).unwrap_or(0);
    if token_power <= 0 {
        return Err(GovernanceError::NoVotingPower);
    }

    let reputation = get_governance_reputation(env, voter.clone());
    let multiplier = 10_000u32.saturating_add(reputation.reputation_score / 2);
    let weighted = checked_mul(token_power, multiplier as i128)? / 10_000;

    let vote = crate::proposals::Vote {
        voter: voter.clone(),
        vote_type: vote_type.clone(),
        voting_power: weighted,
        timestamp: now,
    };

    proposal.voters.set(voter.clone(), vote);
    proposal.voter_list.push_back(voter.clone());
    match vote_type {
        VoteType::For => proposal.votes_for = proposal.votes_for.saturating_add(weighted),
        VoteType::Against => {
            proposal.votes_against = proposal.votes_against.saturating_add(weighted)
        }
        VoteType::Abstain => {
            proposal.votes_abstain = proposal.votes_abstain.saturating_add(weighted)
        }
    }
    if proposal.status == ProposalStatus::Pending {
        proposal.status = ProposalStatus::Active;
    }

    proposals::put_proposal(env, &proposal)?;
    record_vote(env, voter.clone(), proposal_id, vote_type)?;

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("repvote")),
        (
            proposal_id,
            voter,
            token_power,
            weighted,
            multiplier as i128,
        ),
    );

    Ok(())
}

pub fn get_reputation_leaderboard(
    env: &Env,
    limit: u32,
) -> Result<Vec<(Address, u32)>, GovernanceError> {
    let state = get_reputation_state(env);
    if state.users.len() > MAX_REPUTATION_USERS {
        return Err(GovernanceError::IterationLimitExceeded);
    }
    let mut sorted: Vec<(Address, u32)> = Vec::new(env);

    let mut i = 0;
    while i < state.users.len() {
        let user = state.users.get(i).unwrap();
        let rep = state.reputations.get(user.clone()).unwrap();

        let mut inserted = false;
        let mut pos = 0;
        while pos < sorted.len() {
            let existing = sorted.get(pos).unwrap();
            if rep.reputation_score > existing.1 {
                sorted.insert(pos, (user.clone(), rep.reputation_score));
                inserted = true;
                break;
            }
            pos += 1;
        }
        if !inserted {
            sorted.push_back((user.clone(), rep.reputation_score));
        }
        i += 1;
    }

    let mut out: Vec<(Address, u32)> = Vec::new(env);
    let mut idx = 0;
    while idx < sorted.len() && idx < limit {
        out.push_back(sorted.get(idx).unwrap());
        idx += 1;
    }
    Ok(out)
}

pub fn distribute_reputation_rewards(
    env: &Env,
    reward_pool: i128,
) -> Result<Vec<(Address, i128)>, GovernanceError> {
    if reward_pool <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }

    let leaders = get_reputation_leaderboard(env, 100)?;
    let mut total_reputation = 0u32;
    let mut i = 0;
    while i < leaders.len() {
        total_reputation = total_reputation.saturating_add(leaders.get(i).unwrap().1);
        i += 1;
    }

    let mut payouts: Vec<(Address, i128)> = Vec::new(env);
    if total_reputation == 0 {
        return Ok(payouts);
    }

    let mut idx = 0;
    while idx < leaders.len() {
        let (user, score) = leaders.get(idx).unwrap();
        let reward = checked_mul(reward_pool, score as i128)? / total_reputation as i128;
        if reward > 0 {
            payouts.push_back((user.clone(), reward));
        }
        idx += 1;
    }

    Ok(payouts)
}

/// Apply tier-based decay schedule to reputation score.
/// Uses the user's tier to determine decay rate, grace period, and cap.
fn apply_tiered_decay(
    env: &Env,
    score: u32,
    last_activity: u64,
    grace_override: u64,
    tier: &ReputationTier,
) -> u32 {
    let config = get_reputation_config(env);
    if !config.decay_enabled {
        return score;
    }

    let now = env.ledger().timestamp();
    if now <= last_activity {
        return score;
    }
    let inactive_days = (now - last_activity) / 86_400;
    if inactive_days == 0 {
        return score;
    }

    // Determine grace period: use override if set (>0), otherwise tier default
    let grace = if grace_override > 0 {
        grace_override
    } else {
        tier.grace_period_days()
    };

    // Only decay if inactivity exceeds grace period
    if inactive_days <= grace {
        return score;
    }

    let excess_days = inactive_days - grace;
    let capped_days = core::cmp::min(excess_days, tier.max_decay_days());
    let daily_decay_bps = tier.decay_rate_bps();
    let retention_factor = 10_000u32.saturating_sub(daily_decay_bps);

    let mut decayed = score as u64;
    let mut i = 0u64;
    while i < capped_days {
        decayed = (decayed * retention_factor as u64) / 10_000;
        i += 1;
    }
    decayed as u32
}

/// Publicly exposed stale-score refresh: re-evaluates a user's reputation
/// with fresh decay and staleness adjustments, storing the updated score.
pub fn refresh_stale_reputation(env: &Env, user: Address) -> Result<u32, GovernanceError> {
    let mut state = get_reputation_state(env);
    let mut rep = get_governance_reputation(env, user.clone());
    rep.tier = compute_tier(&rep);
    rep.reputation_score = calculate_reputation_score(env, user.clone())?;
    upsert_rep(&mut state, &user, rep);
    put_reputation_state(env, &state);
    Ok(get_governance_reputation(env, user).reputation_score)
}

fn calculate_badge_bonus(badges: &Vec<Badge>) -> u32 {
    let mut bonus = 0u32;
    let mut idx = 0;
    while idx < badges.len() {
        let badge = badges.get(idx).unwrap();
        bonus = bonus.saturating_add(match badge {
            Badge::ActiveVoter(_) => 200,
            Badge::ProposalAuthor(successful_proposals) => {
                min(500, successful_proposals.saturating_mul(100))
            }
            Badge::CommitteeMember(_) => 300,
            Badge::EarlyAdopter => 500,
            Badge::TopDelegator => 400,
            Badge::ConsistentParticipant(months) => min(600, months.saturating_mul(50)),
        });
        idx += 1;
    }
    min(2000, bonus)
}

fn check_and_award_badges(env: &Env, rep: &mut GovernanceReputation) {
    let mut awarded: Vec<String> = Vec::new(env);

    if rep.participation_history.votes_cast >= 50 {
        let badge = Badge::ActiveVoter(50);
        if !contains_badge(&rep.badges, &badge) {
            rep.badges.push_back(badge);
            awarded.push_back(String::from_str(env, "ActiveVoter"));
        }
    }

    if rep.participation_history.proposals_succeeded >= 10 {
        let badge = Badge::ProposalAuthor(10);
        if !contains_badge(&rep.badges, &badge) {
            rep.badges.push_back(badge);
            awarded.push_back(String::from_str(env, "ProposalAuthor"));
        }
    }

    if rep.participation_history.total_tokens_delegated >= 100_000 * PRECISION {
        let badge = Badge::TopDelegator;
        if !contains_badge(&rep.badges, &badge) {
            rep.badges.push_back(badge);
            awarded.push_back(String::from_str(env, "TopDelegator"));
        }
    }

    if !awarded.is_empty() {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("gov"), symbol_short!("badges")),
            (rep.user.clone(), awarded),
        );
    }
}

fn contains_badge(badges: &Vec<Badge>, target: &Badge) -> bool {
    let mut idx = 0;
    while idx < badges.len() {
        if badges.get(idx).unwrap() == *target {
            return true;
        }
        idx += 1;
    }
    false
}

fn upsert_rep(state: &mut ReputationState, user: &Address, rep: GovernanceReputation) {
    if !state.reputations.contains_key(user.clone()) {
        state.users.push_back(user.clone());
    }
    state.reputations.set(user.clone(), rep);
}
