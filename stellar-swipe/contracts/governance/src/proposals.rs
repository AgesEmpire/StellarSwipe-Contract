use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, Map, String, Vec};
use stellar_swipe_common::{sanitize_string, Asset};

use crate::{
    add_balance, checked_add, checked_mul, checked_sub, get_holders, get_staked_balance,
    get_total_supply, get_treasury, get_vote_snapshot, put_treasury, put_vote_snapshots,
    require_admin, GovernanceError, StorageKey,
};

// ── #692: Integer square root (Newton's method, no floating-point) ───────────
//
// #837: the previous version seeded Newton's method with `(x + 1) / 2`. When
// `x == i128::MAX` (a whale's token balance), `x + 1` overflows before the
// division ever runs — panicking in debug builds and wrapping to a bogus,
// tiny voting weight in release builds. Widening to `u128` and seeding with
// `x / 2 + (x & 1)` (an overflow-free way to compute `ceil(x / 2)`) keeps
// every intermediate step comfortably inside range, including at
// `u128::MAX`, which is more headroom than an `i128` balance can ever need.

/// Compute floor(sqrt(n)) using integer arithmetic only. Returns 0 for n <= 0.
pub fn isqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    isqrt_u128(n as u128) as i128
}

/// Overflow-safe floor(sqrt(n)) for the full `u128` range.
fn isqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x / 2 + (x & 1);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ── #693: Proposal Category ──────────────────────────────────────────────────

/// Classification for a governance proposal.  Each category can have its own
/// quorum and supermajority threshold configured by the admin.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalCategory {
    /// Low-risk configuration adjustments (e.g. fee rate tweaks).
    ParameterChange,
    /// High-risk WASM upgrades to any contract.
    ContractUpgrade,
    /// Treasury asset transfers to external addresses.
    TreasuryTransfer,
    /// Catch-all for proposals that don't fit the above categories.
    General,
}

/// Per-category quorum and supermajority overrides. Values are in basis points
/// (10 000 = 100 %). A value of 0 means "use the global GovernanceConfig".
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryThreshold {
    /// Minimum participation as a fraction of total supply (bps, 0 = global).
    pub quorum_bps: u32,
    /// Required for-vote fraction of cast votes (bps, 0 = global).
    pub supermajority_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalType {
    ParameterChange(String, i128, i128),
    TreasurySpend(Address, i128, Asset, String),
    FeatureToggle(String, bool),
    ContractUpgrade(String, Bytes),
    SignalProposal(String),
    Custom(Address),
}

// ── #997: Proposal action allowlist & parameter schema ──────────────────────
//
// `ProposalType` *is* the governance action allowlist: it is a closed Rust
// enum, so the Soroban host rejects any XDR payload that doesn't decode into
// one of the variants below before contract code ever runs, and
// `execute_proposal_action` (and `simulate_execution`) match every variant
// explicitly with no wildcard/default arm — there is no fallback path that
// could execute an action type outside this set.
//
// What enum-typing alone does *not* catch is a well-formed variant carrying
// nonsensical parameters (an out-of-range `ParameterChange` swing, a
// zero-amount `TreasurySpend`, an empty `FeatureToggle` name, a garbage
// all-zero WASM hash for `ContractUpgrade`, ...). `validate_proposal` below
// is the single, centralized place that checks those bounds — for every
// variant, with no catch-all arm — at proposal-creation time, so nothing but
// validated data is ever written to proposal storage.
//
// Client-facing schema (bump [`PROPOSAL_ACTION_SCHEMA_VERSION`] whenever a
// variant, or its bounds, changes):
//
// | Action           | Parameters                             | Bounds enforced at creation |
// |------------------|-----------------------------------------|------------------------------|
// | `ParameterChange`| `(name, current, proposed)`             | `name`: 1..=`MAX_ACTION_STRING_LEN` bytes, no control chars. `current`/`proposed`: within `±MAX_PARAMETER_VALUE`. If `current > 0`: `abs(proposed - current) * 2 < current`. |
// | `TreasurySpend`  | `(recipient, amount, asset, purpose)`   | `0 < amount <= treasury_balance(asset)` and `amount * 10 <= treasury_balance(asset)`. `purpose`: ≤ `MAX_ACTION_STRING_LEN` bytes, no control chars. |
// | `FeatureToggle`  | `(feature, enabled)`                    | `feature`: 1..=`MAX_ACTION_STRING_LEN` bytes, no control chars. |
// | `ContractUpgrade`| `(contract_name, wasm_hash)`            | `contract_name`: 1..=`MAX_ACTION_STRING_LEN` bytes, no control chars. `wasm_hash`: exactly 32 bytes and not all-zero. |
// | `SignalProposal` | `(text)`                                | `text`: 1..=`MAX_ACTION_STRING_LEN` bytes, no control chars. |
// | `Custom`         | `(executor)`                            | `executor`: a host-validated `Address`. `execution_payload` (separate field) must be non-empty. |

/// Bumped whenever the set of supported [`ProposalType`] variants, or the
/// parameter bounds enforced on them by [`validate_proposal`], changes.
/// Off-chain clients should treat a decrease, or an unrecognized bump, as a
/// signal to re-check their proposal-building logic against this schema.
pub const PROPOSAL_ACTION_SCHEMA_VERSION: u32 = 1;

/// Inclusive bound (in either direction) on any raw `i128` parameter carried
/// by a proposal action, e.g. `ParameterChange`'s `current`/`proposed`.
/// Chosen far above any realistic on-chain quantity while leaving enormous
/// headroom below `i128::MAX`/`i128::MIN`, so bounded values can never
/// overflow the arithmetic `validate_proposal` performs on them.
pub const MAX_PARAMETER_VALUE: i128 = 1_000_000_000_000_000_000; // 10^18

/// Upper bound on the byte length of any free-text identifier carried by a
/// proposal action (parameter/feature/contract names, signal text, spend
/// purpose).
pub const MAX_ACTION_STRING_LEN: u32 = 256;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    Pending,
    Active,
    Succeeded,
    Failed,
    Executed,
    Cancelled,
    Expired,
    /// Proposal was voluntarily withdrawn by the original proposer before
    /// voting opened.  This status is immutable once set — the proposal is
    /// permanently excluded from future voting-eligible listings.
    Withdrawn,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoteType {
    For,
    Against,
    Abstain,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Vote {
    pub voter: Address,
    pub vote_type: VoteType,
    pub voting_power: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub proposal_type: ProposalType,
    pub title: String,
    pub description: String,
    pub execution_payload: Bytes,
    pub voting_starts: u64,
    pub voting_ends: u64,
    pub votes_for: i128,
    pub votes_against: i128,
    pub votes_abstain: i128,
    pub status: ProposalStatus,
    pub voters: Map<Address, Vote>,
    pub voter_list: Vec<Address>,
    pub executed_at: Option<u64>,
    // ── #667: Mandatory discussion period ────────────────────────────────────
    /// Timestamp after which votes are accepted. Zero means no discussion window.
    pub discussion_ends_at: u64,
    // ── #693: Category classification ────────────────────────────────────────
    /// Category used to select per-category quorum/supermajority thresholds.
    pub category: ProposalCategory,
    // ── #692: Per-proposal quadratic voting flag ──────────────────────────────
    /// When `true`, each voter's effective weight is floor(sqrt(staked_balance))
    /// instead of the raw staked balance.
    pub use_quadratic_voting: bool,
    /// Sum of floor(sqrt(p)) for all snapshotted holders at proposal creation.
    /// Used as the quadratic-adjusted total supply for quorum checks.
    pub quadratic_total_supply: i128,
    // ── #796: Treasury spend proposal execution expiry ───────────────────────
    /// Ledger timestamp after which a `Succeeded` proposal can no longer be
    /// executed. Set at creation as `voting_ends + EXECUTION_WINDOW`. Prevents
    /// approved-but-unexecuted spend authorisations from locking treasury
    /// funds indefinitely — once past this deadline, `execute_proposal`
    /// rejects with `GovernanceError::ProposalExpired` and any DAO member may
    /// call `reclaim_expired_proposal` to clear the entry.
    pub execution_deadline: u64,
}

/// Execution window after `voting_ends` during which a `Succeeded` proposal
/// may still be executed (Issue #796). Mirrors the 7-day period used
/// elsewhere in this module for the default voting period.
pub const EXECUTION_WINDOW: u64 = 7 * 24 * 60 * 60;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovernanceConfig {
    pub min_proposal_threshold: i128,
    pub voting_period: u64,
    pub voting_delay: u64,
    pub quorum_threshold: u32,
    pub approval_threshold: u32,
    pub execution_delay: u64,
    /// Mandatory discussion window (seconds) before votes can be cast (Issue #667).
    /// A value of 0 disables the discussion period.
    pub discussion_duration: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalStatistics {
    pub total_proposals: u32,
    pub active_proposals: u32,
    pub succeeded_proposals: u32,
    pub failed_proposals: u32,
    pub executed_proposals: u32,
    pub avg_participation_rate: u32,
    pub avg_approval_rate: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteDelegation {
    pub delegator: Address,
    pub delegate: Address,
    pub delegated_power: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegationState {
    pub delegations: Map<Address, VoteDelegation>,
    pub delegators: Vec<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalsState {
    pub proposals: Map<u64, Proposal>,
    pub proposal_ids: Vec<u64>,
    pub next_proposal_id: u64,
}

const BPS_DENOMINATOR: i128 = 10_000;

pub fn default_governance_config() -> GovernanceConfig {
    GovernanceConfig {
        min_proposal_threshold: 1_000,
        voting_period: 7 * 24 * 60 * 60,
        voting_delay: 60,
        quorum_threshold: 1_000,
        approval_threshold: 5_000,
        execution_delay: 0,
        discussion_duration: 0,
    }
}

pub fn empty_proposals_state(env: &Env) -> ProposalsState {
    ProposalsState {
        proposals: Map::new(env),
        proposal_ids: Vec::new(env),
        next_proposal_id: 1,
    }
}

pub fn empty_delegation_state(env: &Env) -> DelegationState {
    DelegationState {
        delegations: Map::new(env),
        delegators: Vec::new(env),
    }
}

pub fn get_governance_config(env: &Env) -> GovernanceConfig {
    env.storage()
        .instance()
        .get(&StorageKey::GovernanceConfig)
        .unwrap_or_else(default_governance_config)
}

pub fn configure_governance(
    env: &Env,
    admin: &Address,
    config: GovernanceConfig,
) -> Result<GovernanceConfig, GovernanceError> {
    require_admin(env, admin)?;
    if config.min_proposal_threshold <= 0
        || config.voting_period == 0
        || config.quorum_threshold > 10_000
        || config.approval_threshold > 10_000
    {
        return Err(GovernanceError::InvalidGovernanceConfig);
    }
    env.storage()
        .instance()
        .set(&StorageKey::GovernanceConfig, &config);
    Ok(config)
}

pub fn get_proposals_state(env: &Env) -> ProposalsState {
    env.storage()
        .instance()
        .get(&StorageKey::ProposalsState)
        .unwrap_or_else(|| empty_proposals_state(env))
}

pub fn put_proposals_state(env: &Env, state: &ProposalsState) {
    env.storage()
        .instance()
        .set(&StorageKey::ProposalsState, state);
}

pub fn get_delegation_state(env: &Env) -> DelegationState {
    env.storage()
        .instance()
        .get(&StorageKey::Delegations)
        .unwrap_or_else(|| empty_delegation_state(env))
}

pub fn put_delegation_state(env: &Env, state: &DelegationState) {
    env.storage()
        .instance()
        .set(&StorageKey::Delegations, state);
}

pub fn create_proposal(
    env: &Env,
    proposer: Address,
    proposal_type: ProposalType,
    title: String,
    description: String,
    execution_payload: Bytes,
    category: ProposalCategory,
    use_quadratic_voting: bool,
) -> Result<u64, GovernanceError> {
    proposer.require_auth();
    if title.is_empty() || description.is_empty() {
        return Err(GovernanceError::InvalidProposal);
    }
    sanitize_string(env, &title, 256).map_err(|_| GovernanceError::InvalidProposal)?;
    sanitize_string(env, &description, 4096).map_err(|_| GovernanceError::InvalidProposal)?;

    let config = get_governance_config(env);
    let power = get_effective_voting_power(env, &proposer);
    if power < config.min_proposal_threshold {
        return Err(GovernanceError::NoVotingPower);
    }

    validate_proposal(env, &proposal_type)?;
    validate_execution_payload(&proposal_type, &execution_payload)?;

    let mut state = get_proposals_state(env);
    let id = state.next_proposal_id;

    // Snapshot every current holder's effective voting power so that staking
    // or unstaking after this point cannot affect votes on this proposal.
    let holders = get_holders(env);
    let mut snapshots: Map<Address, i128> = Map::new(env);
    // #692: accumulate the quadratic total supply at snapshot time.
    let mut quadratic_total_supply: i128 = 0;
    let mut hi = 0;
    while hi < holders.len() {
        let h = holders.get(hi).unwrap();
        let p = get_effective_voting_power(env, &h);
        if p > 0 {
            snapshots.set(h, p);
            if use_quadratic_voting {
                quadratic_total_supply = quadratic_total_supply.saturating_add(isqrt(p));
            }
        }
        hi += 1;
    }
    put_vote_snapshots(env, id, &snapshots);
    let now = env.ledger().timestamp();

    let discussion_ends_at = if config.discussion_duration > 0 {
        now.saturating_add(config.discussion_duration)
    } else {
        0
    };

    let voting_ends = now
        .saturating_add(config.voting_delay)
        .saturating_add(config.voting_period);

    let proposal = Proposal {
        id,
        proposer: proposer.clone(),
        proposal_type,
        title,
        description,
        execution_payload,
        voting_starts: now.saturating_add(config.voting_delay),
        voting_ends,
        votes_for: 0,
        votes_against: 0,
        votes_abstain: 0,
        status: ProposalStatus::Pending,
        voters: Map::new(env),
        voter_list: Vec::new(env),
        executed_at: None,
        discussion_ends_at,
        category,
        use_quadratic_voting,
        quadratic_total_supply,
        execution_deadline: voting_ends.saturating_add(EXECUTION_WINDOW),
    };

    state.proposals.set(id, proposal.clone());
    state.proposal_ids.push_back(id);
    state.next_proposal_id = id.saturating_add(1);
    put_proposals_state(env, &state);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("propnew")),
        (
            id,
            proposer,
            proposal.discussion_ends_at,
            proposal.voting_starts,
            proposal.voting_ends,
        ),
    );

    Ok(id)
}

pub fn get_proposal(env: &Env, proposal_id: u64) -> Result<Proposal, GovernanceError> {
    get_proposals_state(env)
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)
}

/// #796: `execute_proposal` cannot persist a `status = Expired` write when it
/// rejects a past-deadline execution — Soroban rolls back all storage writes
/// made during an invocation that returns `Err`, so that assignment is a
/// no-op in practice. Callers that need to *observe* the effective status
/// (as opposed to internal state-machine code, which gates on the raw stored
/// status) should use this instead of the raw stored value, mirroring the
/// deadline check `get_active_proposals` already does independently of it.
pub fn effective_status(env: &Env, proposal: &Proposal) -> ProposalStatus {
    if proposal.status == ProposalStatus::Succeeded
        && env.ledger().timestamp() > proposal.execution_deadline
    {
        ProposalStatus::Expired
    } else {
        proposal.status.clone()
    }
}

pub fn put_proposal(env: &Env, proposal: &Proposal) -> Result<(), GovernanceError> {
    let mut state = get_proposals_state(env);
    if !state.proposals.contains_key(proposal.id) {
        return Err(GovernanceError::ProposalNotFound);
    }
    state.proposals.set(proposal.id, proposal.clone());
    put_proposals_state(env, &state);
    Ok(())
}

pub fn cast_vote(
    env: &Env,
    proposal_id: u64,
    voter: Address,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    voter.require_auth();
    let mut proposal = get_proposal(env, proposal_id)?;
    let now = env.ledger().timestamp();

    // ── #667: Enforce discussion period ──────────────────────────────────────
    if proposal.discussion_ends_at > 0 && now < proposal.discussion_ends_at {
        return Err(GovernanceError::DiscussionPeriodActive);
    }

    // Emit a one-time event when the proposal first transitions out of the
    // discussion phase (voter_list is still empty on the first accepted vote).
    if proposal.discussion_ends_at > 0 && proposal.voter_list.is_empty() {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("gov"), symbol_short!("discend")),
            (proposal_id, proposal.discussion_ends_at, now),
        );
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

    let raw_power = get_vote_snapshot(env, proposal_id, &voter).unwrap_or(0);
    if raw_power <= 0 {
        return Err(GovernanceError::NoVotingPower);
    }

    // #692: when quadratic voting is enabled the effective weight is
    // floor(sqrt(staked_balance)) rather than the raw balance.
    let power = if proposal.use_quadratic_voting {
        let q = isqrt(raw_power);
        if q <= 0 {
            return Err(GovernanceError::NoVotingPower);
        }
        q
    } else {
        raw_power
    };

    let vote = Vote {
        voter: voter.clone(),
        vote_type: vote_type.clone(),
        voting_power: power,
        timestamp: now,
    };
    proposal.voters.set(voter.clone(), vote);
    proposal.voter_list.push_back(voter.clone());

    match vote_type {
        VoteType::For => proposal.votes_for = checked_add(proposal.votes_for, power)?,
        VoteType::Against => proposal.votes_against = checked_add(proposal.votes_against, power)?,
        VoteType::Abstain => proposal.votes_abstain = checked_add(proposal.votes_abstain, power)?,
    }

    if proposal.status == ProposalStatus::Pending {
        proposal.status = ProposalStatus::Active;
    }
    put_proposal(env, &proposal)
}

pub fn finalize_proposal(env: &Env, proposal_id: u64) -> Result<ProposalStatus, GovernanceError> {
    let mut proposal = get_proposal(env, proposal_id)?;
    if env.ledger().timestamp() < proposal.voting_ends {
        return Err(GovernanceError::InvalidDuration);
    }
    if proposal.status != ProposalStatus::Pending && proposal.status != ProposalStatus::Active {
        return Err(GovernanceError::ProposalNotActive);
    }

    let cfg = get_governance_config(env);

    // #693: look up category-specific thresholds, fall back to global config.
    let (quorum_bps, approval_bps) = resolve_category_thresholds(env, &proposal.category, &cfg);

    let total_votes = proposal
        .votes_for
        .saturating_add(proposal.votes_against)
        .saturating_add(proposal.votes_abstain);

    // #692: when quadratic voting is active, compare against the quadratic
    // total supply stored at proposal creation rather than the linear supply.
    let reference_supply = if proposal.use_quadratic_voting && proposal.quadratic_total_supply > 0 {
        proposal.quadratic_total_supply
    } else {
        let ts = get_total_supply(env)?;
        if ts <= 0 {
            return Err(GovernanceError::InvalidSupply);
        }
        ts
    };

    let quorum_met = total_votes.saturating_mul(BPS_DENOMINATOR)
        >= reference_supply.saturating_mul(quorum_bps as i128);

    if !quorum_met {
        proposal.status = ProposalStatus::Failed;
        put_proposal(env, &proposal)?;
        return Ok(ProposalStatus::Failed);
    }

    let cast_votes = proposal.votes_for.saturating_add(proposal.votes_against);
    let approved = cast_votes > 0
        && proposal.votes_for.saturating_mul(BPS_DENOMINATOR)
            >= cast_votes.saturating_mul(approval_bps as i128);

    proposal.status = if approved {
        ProposalStatus::Succeeded
    } else {
        ProposalStatus::Failed
    };
    let status = proposal.status.clone();

    if status == ProposalStatus::Succeeded {
        if let ProposalType::ContractUpgrade(ref contract, ref new_hash) = proposal.proposal_type {
            let execution_available_after =
                proposal.voting_ends.saturating_add(cfg.execution_delay);
            env.events().publish(
                (symbol_short!("upgrade"), symbol_short!("announced")),
                (
                    contract.clone(),
                    new_hash.clone(),
                    execution_available_after,
                    proposal.execution_payload.clone(),
                ),
            );
        }
    }

    put_proposal(env, &proposal)?;

    if status == ProposalStatus::Succeeded && cfg.execution_delay == 0 {
        let _ = execute_proposal(env, proposal_id, proposal.proposer.clone());
    }

    Ok(status)
}

pub fn execute_proposal(
    env: &Env,
    proposal_id: u64,
    executor: Address,
) -> Result<ProposalStatus, GovernanceError> {
    executor.require_auth();
    let mut proposal = get_proposal(env, proposal_id)?;
    if proposal.status != ProposalStatus::Succeeded {
        return Err(GovernanceError::ProposalNotApproved);
    }

    let ready = proposal
        .voting_ends
        .saturating_add(get_governance_config(env).execution_delay);
    let now = env.ledger().timestamp();
    if now < ready {
        return Err(GovernanceError::InvalidDuration);
    }

    // ── #796: Reject execution once the proposal's execution window has
    // closed. Approved-but-unexecuted proposals must be reclaimed instead of
    // executed once expired, so treasury funds don't stay conceptually locked
    // forever behind a stale authorisation.
    //
    // Note: an `Err`-returning invocation rolls back every storage write it
    // made, so there is no point writing `status = Expired` here — it would
    // never persist. `effective_status` computes it lazily for readers
    // instead; `reclaim_expired_proposal` already tolerates the stored status
    // still reading `Succeeded` past the deadline.
    if now > proposal.execution_deadline {
        return Err(GovernanceError::ProposalExpired);
    }

    execute_proposal_action(env, &proposal)?;
    proposal.status = ProposalStatus::Executed;
    proposal.executed_at = Some(env.ledger().timestamp());
    put_proposal(env, &proposal)?;
    Ok(ProposalStatus::Executed)
}

pub fn execute_proposal_action(env: &Env, proposal: &Proposal) -> Result<(), GovernanceError> {
    match &proposal.proposal_type {
        ProposalType::ParameterChange(parameter, _current, proposed) => {
            let mut params: Map<String, i128> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceParameters)
                .unwrap_or(Map::new(env));
            params.set(parameter.clone(), *proposed);
            env.storage()
                .instance()
                .set(&StorageKey::GovernanceParameters, &params);
        }
        ProposalType::TreasurySpend(recipient, amount, asset, _purpose) => {
            let mut treasury = get_treasury(env);
            let bal = treasury.assets.get(asset.clone()).unwrap_or(0);
            if bal < *amount {
                return Err(GovernanceError::InsufficientBalance);
            }
            treasury
                .assets
                .set(asset.clone(), checked_sub(bal, *amount)?);
            put_treasury(env, &treasury);
            add_balance(env, recipient, *amount)?;
        }
        ProposalType::FeatureToggle(feature, enabled) => {
            let mut flags: Map<String, bool> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceFeatures)
                .unwrap_or(Map::new(env));
            flags.set(feature.clone(), *enabled);
            env.storage()
                .instance()
                .set(&StorageKey::GovernanceFeatures, &flags);
        }
        ProposalType::ContractUpgrade(contract_name, new_hash) => {
            let mut upgrades: Map<String, Bytes> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceUpgrades)
                .unwrap_or(Map::new(env));
            upgrades.set(contract_name.clone(), new_hash.clone());
            env.storage()
                .instance()
                .set(&StorageKey::GovernanceUpgrades, &upgrades);
        }
        ProposalType::SignalProposal(_) => {}
        ProposalType::Custom(_) => {}
    }
    Ok(())
}

pub fn execute_proposal_action_by_id(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let proposal = get_proposal(env, proposal_id)?;
    execute_proposal_action(env, &proposal)
}

pub fn mark_proposal_executed(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let mut proposal = get_proposal(env, proposal_id)?;
    proposal.status = ProposalStatus::Executed;
    proposal.executed_at = Some(env.ledger().timestamp());
    put_proposal(env, &proposal)
}

pub fn cancel_proposal(
    env: &Env,
    proposal_id: u64,
    canceller: Address,
) -> Result<ProposalStatus, GovernanceError> {
    canceller.require_auth();
    let mut proposal = get_proposal(env, proposal_id)?;
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    let guardian_ok = env
        .storage()
        .instance()
        .get::<_, Address>(&StorageKey::Guardian)
        .map(|g| g == canceller)
        .unwrap_or(false);

    if canceller != proposal.proposer && canceller != admin && !guardian_ok {
        return Err(GovernanceError::Unauthorized);
    }
    if proposal.status == ProposalStatus::Executed {
        return Err(GovernanceError::InvalidCommitteeAction);
    }

    proposal.status = ProposalStatus::Cancelled;
    put_proposal(env, &proposal)?;
    Ok(ProposalStatus::Cancelled)
}

/// Voluntarily withdraw a proposal by its original proposer.
///
/// # Behaviour
/// - Only the original proposer may call this entrypoint (authorization check).
/// - Only allowed while the proposal is in `Pending` status (pre-vote state).
///   Once voting has opened (status transitions to `Active`) or the proposal
///   has reached any other terminal state, withdrawal is rejected.
/// - On success:
///   1. Status is set to `Withdrawn` (immutable — cannot be further modified).
///   2. The spam-deposit is **refunded** to the proposer (区别 from failed
///      proposals which forfeit the deposit). The rationale is that a
///      responsible self-withdrawal (e.g. correcting a mistake) should not
///      penalise the proposer, unlike a proposal that fails due to lack of
///      community support.
///   3. A `propwdr` event is emitted with the proposer, proposal ID, and
///      timestamp.
///
/// # Deposit Handling Policy
/// Self-withdrawal **always refunds** the deposit, regardless of participation
/// thresholds. This distinguishes it from `finalize_proposal` where a failed
/// proposal forfeits its deposit to the treasury. The reasoning:
/// - A proposer who self-withdraws is acting responsibly (e.g. fixing errors).
/// - Penalising responsible behaviour would discourage honest participation.
/// - The spam-deposit's purpose (deterring frivolous proposals) is served by
///   the forfeit path for proposals that actually fail, not by penalising
///   voluntary corrections.
///
/// # Errors
/// - [`GovernanceError::Unauthorized`] — caller is not the original proposer.
/// - [`GovernanceError::ProposalNotFound`] — proposal_id does not exist.
/// - [`GovernanceError::ProposalNotActive`] — proposal is not in Pending status
///   (voting has already started or the proposal reached a terminal state).
pub fn withdraw_proposal(
    env: &Env,
    proposal_id: u64,
    proposer: Address,
) -> Result<ProposalStatus, GovernanceError> {
    proposer.require_auth();
    let mut proposal = get_proposal(env, proposal_id)?;

    // Only the original proposer may withdraw their own proposal.
    if proposer != proposal.proposer {
        return Err(GovernanceError::Unauthorized);
    }

    // Must still be in Pending status — once voting opens (Active) or any
    // terminal state is reached, withdrawal is no longer permitted.
    if proposal.status == ProposalStatus::Withdrawn {
        return Err(GovernanceError::ProposalAlreadyWithdrawn);
    }
    if proposal.status != ProposalStatus::Pending {
        return Err(GovernanceError::ProposalNotActive);
    }

    proposal.status = ProposalStatus::Withdrawn;
    put_proposal(env, &proposal)?;

    // Refund the spam-deposit to the proposer (see deposit handling policy above).
    // Directly refund via add_balance and clean up the lock record.
    let config = crate::proposal_deposit::get_deposit_config(env);
    if config.amount > 0 {
        // Check if a deposit was locked for this proposal.
        let locked: Option<Address> =
            env.storage()
                .persistent()
                .get(&crate::proposal_deposit::DepositKey::LockedDeposit(
                    proposal_id,
                ));
        if let Some(deposit_proposer) = locked {
            if deposit_proposer == proposer {
                // Refund the full deposit amount to the proposer.
                let _ = crate::add_balance(env, &proposer, config.amount);
                env.storage().persistent().remove(
                    &crate::proposal_deposit::DepositKey::LockedDeposit(proposal_id),
                );
                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("deposit"), symbol_short!("refund")),
                    (proposal_id, proposer.clone(), config.amount),
                );
            }
        }
    }

    // Emit self-withdrawal event.
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("propwdr")),
        (proposal_id, proposer, env.ledger().timestamp()),
    );

    Ok(ProposalStatus::Withdrawn)
}

pub fn calculate_proposal_statistics(env: &Env) -> Result<ProposalStatistics, GovernanceError> {
    let state = get_proposals_state(env);
    let total_supply = get_total_supply(env)?;

    let mut total = 0u32;
    let mut active = 0u32;
    let mut succeeded = 0u32;
    let mut failed = 0u32;
    let mut executed = 0u32;
    let mut part_total = 0u64;
    let mut part_count = 0u32;
    let mut appr_total = 0u64;
    let mut appr_count = 0u32;

    let mut i = 0;
    while i < state.proposal_ids.len() {
        let id = state.proposal_ids.get(i).unwrap();
        if let Some(p) = state.proposals.get(id) {
            total = total.saturating_add(1);
            match p.status {
                ProposalStatus::Pending | ProposalStatus::Active => {
                    active = active.saturating_add(1)
                }
                ProposalStatus::Succeeded => succeeded = succeeded.saturating_add(1),
                ProposalStatus::Failed => failed = failed.saturating_add(1),
                ProposalStatus::Executed => executed = executed.saturating_add(1),
                _ => {}
            }

            let all_votes = p
                .votes_for
                .saturating_add(p.votes_against)
                .saturating_add(p.votes_abstain);
            if total_supply > 0 {
                part_total = part_total.saturating_add(
                    (all_votes.saturating_mul(BPS_DENOMINATOR) / total_supply) as u64,
                );
                part_count = part_count.saturating_add(1);
            }

            let cast_votes = p.votes_for.saturating_add(p.votes_against);
            if cast_votes > 0 {
                appr_total = appr_total.saturating_add(
                    (p.votes_for.saturating_mul(BPS_DENOMINATOR) / cast_votes) as u64,
                );
                appr_count = appr_count.saturating_add(1);
            }
        }
        i += 1;
    }

    Ok(ProposalStatistics {
        total_proposals: total,
        active_proposals: active,
        succeeded_proposals: succeeded,
        failed_proposals: failed,
        executed_proposals: executed,
        avg_participation_rate: if part_count > 0 {
            (part_total / part_count as u64) as u32
        } else {
            0
        },
        avg_approval_rate: if appr_count > 0 {
            (appr_total / appr_count as u64) as u32
        } else {
            0
        },
    })
}

pub fn get_all_proposals(env: &Env) -> Vec<Proposal> {
    let state = get_proposals_state(env);
    let mut out = Vec::new(env);
    let mut i = 0;
    while i < state.proposal_ids.len() {
        let id = state.proposal_ids.get(i).unwrap();
        if let Some(p) = state.proposals.get(id) {
            out.push_back(p);
        }
        i += 1;
    }
    out
}

// ── #796: Treasury spend proposal execution expiry ───────────────────────────

/// Reclaim a `Succeeded` proposal whose execution window has closed without
/// ever being executed. Callable by **any** address (not admin-only) — any
/// DAO member can clear a stale authorisation once it has expired. Removes
/// the proposal entry entirely and emits a `TreasuryProposalExpired` event.
///
/// # Errors
/// - [`GovernanceError::ProposalNotFound`] — `proposal_id` does not exist.
/// - [`GovernanceError::ProposalNotApproved`] — proposal never reached
///   `Succeeded` status (nothing to reclaim).
/// - [`GovernanceError::InvalidDuration`] — the execution window has not
///   closed yet; the proposal can still be executed normally.
pub fn reclaim_expired_proposal(
    env: &Env,
    proposal_id: u64,
    caller: Address,
) -> Result<(), GovernanceError> {
    caller.require_auth();

    let mut state = get_proposals_state(env);
    let proposal = state
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    // A proposal is reclaimable once it succeeded but was never executed —
    // whether its status is still `Succeeded` or a prior `execute_proposal`
    // call already flipped it to `Expired` after the deadline passed.
    if proposal.status != ProposalStatus::Succeeded && proposal.status != ProposalStatus::Expired {
        return Err(GovernanceError::ProposalNotApproved);
    }
    if env.ledger().timestamp() <= proposal.execution_deadline {
        return Err(GovernanceError::InvalidDuration);
    }

    state.proposals.remove(proposal_id);
    remove_proposal_id(&mut state.proposal_ids, proposal_id);
    put_proposals_state(env, &state);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("treasury"), symbol_short!("propexp")),
        (
            proposal_id,
            proposal.proposer,
            caller,
            env.ledger().timestamp(),
        ),
    );

    Ok(())
}

/// Proposals that are still eligible for voting or execution: `Pending`,
/// `Active`, or `Succeeded` (awaiting execution) *and* not past their
/// `execution_deadline`. Proposals whose execution window has closed are
/// excluded even if their on-chain status hasn't been transitioned to
/// `Expired` yet (Issue #796).
pub fn get_active_proposals(env: &Env) -> Vec<Proposal> {
    let now = env.ledger().timestamp();
    let all = get_all_proposals(env);
    let mut out = Vec::new(env);
    let mut i = 0;
    while i < all.len() {
        let p = all.get(i).unwrap();
        let active_status = matches!(
            p.status,
            ProposalStatus::Pending | ProposalStatus::Active | ProposalStatus::Succeeded
        );
        if active_status && now <= p.execution_deadline {
            out.push_back(p);
        }
        i += 1;
    }
    out
}

fn remove_proposal_id(ids: &mut Vec<u64>, target: u64) {
    let mut i = 0;
    while i < ids.len() {
        if ids.get(i).unwrap() == target {
            ids.remove(i);
            return;
        }
        i += 1;
    }
}

pub fn delegate_voting_power(
    env: &Env,
    delegator: Address,
    delegate: Address,
) -> Result<(), GovernanceError> {
    delegator.require_auth();
    if delegator == delegate {
        return Err(GovernanceError::InvalidProposal);
    }

    let mut state = get_delegation_state(env);
    if state
        .delegations
        .get(delegator.clone())
        .map(|d| d.active)
        .unwrap_or(false)
    {
        return Err(GovernanceError::InvalidCommitteeAction);
    }

    let power = get_staked_balance(env, &delegator);
    if power <= 0 {
        return Err(GovernanceError::NoVotingPower);
    }

    state.delegations.set(
        delegator.clone(),
        VoteDelegation {
            delegator: delegator.clone(),
            delegate,
            delegated_power: power,
            active: true,
        },
    );
    if !contains_address(&state.delegators, &delegator) {
        state.delegators.push_back(delegator);
    }
    put_delegation_state(env, &state);
    Ok(())
}

pub fn undelegate_voting_power(env: &Env, delegator: Address) -> Result<(), GovernanceError> {
    delegator.require_auth();
    let mut state = get_delegation_state(env);
    let mut d = state
        .delegations
        .get(delegator.clone())
        .ok_or(GovernanceError::CrossCommitteeRequestNotFound)?;
    d.active = false;
    state.delegations.set(delegator, d);
    put_delegation_state(env, &state);
    Ok(())
}

pub fn get_effective_voting_power(env: &Env, user: &Address) -> i128 {
    let state = get_delegation_state(env);
    let own = if state
        .delegations
        .get(user.clone())
        .map(|d| d.active)
        .unwrap_or(false)
    {
        0
    } else {
        get_staked_balance(env, user)
    };

    let mut delegated = 0i128;
    let mut i = 0;
    while i < state.delegators.len() {
        let delegator = state.delegators.get(i).unwrap();
        if let Some(d) = state.delegations.get(delegator) {
            if d.active && d.delegate == *user {
                delegated = delegated.saturating_add(d.delegated_power);
            }
        }
        i += 1;
    }

    own.saturating_add(delegated)
}

/// Validate a proposal's action and its embedded parameters against the
/// allowlisted schema documented above `PROPOSAL_ACTION_SCHEMA_VERSION`.
///
/// This match is intentionally exhaustive with **no wildcard arm**: adding a
/// new `ProposalType` variant without extending this function is a compile
/// error, so a new action type can never silently skip validation.
fn validate_proposal(env: &Env, p: &ProposalType) -> Result<(), GovernanceError> {
    match p {
        ProposalType::ParameterChange(parameter, current, proposed) => {
            if parameter.is_empty() {
                return Err(GovernanceError::InvalidProposal);
            }
            sanitize_string(env, parameter, MAX_ACTION_STRING_LEN)
                .map_err(|_| GovernanceError::InvalidProposal)?;
            if !is_within_parameter_bounds(*current) || !is_within_parameter_bounds(*proposed) {
                return Err(GovernanceError::ProposalParameterOutOfRange);
            }
            if *current > 0 {
                // Safe: both operands are bounded to ±MAX_PARAMETER_VALUE
                // above, so the subtraction and doubling below cannot
                // overflow i128 (unlike the raw `-`/`.abs()` this replaced,
                // which panicked on an unbounded `current == i128::MIN`).
                let delta = checked_sub(*proposed, *current)?.abs();
                if checked_mul(delta, 2)? >= *current {
                    return Err(GovernanceError::InvalidProposal);
                }
            }
        }
        ProposalType::TreasurySpend(_recipient, amount, asset, purpose) => {
            sanitize_string(env, purpose, MAX_ACTION_STRING_LEN)
                .map_err(|_| GovernanceError::InvalidProposal)?;
            let treasury = get_treasury(env);
            let bal = treasury.assets.get(asset.clone()).unwrap_or(0);
            if *amount <= 0 || *amount > bal || amount.saturating_mul(10) > bal {
                return Err(GovernanceError::BudgetExceeded);
            }
        }
        ProposalType::FeatureToggle(feature, _enabled) => {
            if feature.is_empty() {
                return Err(GovernanceError::InvalidProposal);
            }
            sanitize_string(env, feature, MAX_ACTION_STRING_LEN)
                .map_err(|_| GovernanceError::InvalidProposal)?;
        }
        ProposalType::ContractUpgrade(name, hash) => {
            if name.is_empty() {
                return Err(GovernanceError::InvalidProposal);
            }
            sanitize_string(env, name, MAX_ACTION_STRING_LEN)
                .map_err(|_| GovernanceError::InvalidProposal)?;
            if hash.len() != 32 {
                return Err(GovernanceError::InvalidProposal);
            }
            if is_all_zero_bytes(hash) {
                return Err(GovernanceError::InvalidProposal);
            }
        }
        ProposalType::SignalProposal(text) => {
            if text.is_empty() {
                return Err(GovernanceError::InvalidProposal);
            }
            sanitize_string(env, text, MAX_ACTION_STRING_LEN)
                .map_err(|_| GovernanceError::InvalidProposal)?;
        }
        ProposalType::Custom(_executor) => {
            // The executor `Address` is validated by the Soroban host at
            // deserialization time (it cannot be malformed by construction).
            // The remaining constraint — a non-empty ABI payload — is
            // enforced by `validate_execution_payload`.
        }
    }
    Ok(())
}

/// `true` when `value` falls within `[-MAX_PARAMETER_VALUE, MAX_PARAMETER_VALUE]`.
fn is_within_parameter_bounds(value: i128) -> bool {
    value >= -MAX_PARAMETER_VALUE && value <= MAX_PARAMETER_VALUE
}

/// `true` when every byte in `bytes` is `0x00` (an obviously-placeholder /
/// malformed WASM hash — a real hash is effectively never all-zero).
fn is_all_zero_bytes(bytes: &Bytes) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i).unwrap_or(1) != 0 {
            return false;
        }
        i += 1;
    }
    true
}

/// Validate the execution payload bytes against what each ProposalType
/// expects, per the schema documented above `PROPOSAL_ACTION_SCHEMA_VERSION`.
///
/// - `ContractUpgrade`: payload must be exactly 32 bytes (new WASM hash).
/// - `TreasurySpend`/`ParameterChange`: non-empty payload must start with a
///   known version byte (`0x01`) so malformed blobs are caught early.
/// - `FeatureToggle`/`SignalProposal`: no payload constraints.
/// - `Custom`: payload must be non-empty (executor address ABI).
///
/// Exhaustive over every `ProposalType` variant with no wildcard arm, so a
/// newly-added action type must be given an explicit payload rule here
/// before it compiles.
pub fn validate_execution_payload(
    proposal_type: &ProposalType,
    payload: &Bytes,
) -> Result<(), GovernanceError> {
    match proposal_type {
        ProposalType::ContractUpgrade(_, _) => {
            // Payload encodes the new WASM hash — must be exactly 32 bytes.
            if payload.len() != 32 {
                return Err(GovernanceError::InvalidProposal);
            }
        }
        ProposalType::TreasurySpend(_, _, _, _) | ProposalType::ParameterChange(_, _, _) => {
            // Optional but if present must carry a recognized version prefix.
            if payload.len() > 0 && payload.get(0) != Some(0x01) {
                return Err(GovernanceError::InvalidProposal);
            }
        }
        ProposalType::Custom(_) => {
            // Custom proposals must supply a non-empty payload (ABI data).
            if payload.is_empty() {
                return Err(GovernanceError::InvalidProposal);
            }
        }
        ProposalType::FeatureToggle(_, _) | ProposalType::SignalProposal(_) => {
            // No payload constraints for these action types.
        }
    }
    Ok(())
}

fn contains_address(list: &Vec<Address>, target: &Address) -> bool {
    let mut i = 0;
    while i < list.len() {
        if list.get(i).unwrap() == *target {
            return true;
        }
        i += 1;
    }
    false
}

// ── #693: Category threshold helpers ─────────────────────────────────────────

/// Return `(quorum_bps, supermajority_bps)` for the given category, falling
/// back to the global [`GovernanceConfig`] when no per-category override exists
/// or when the stored values are 0.
pub fn resolve_category_thresholds(
    env: &Env,
    category: &ProposalCategory,
    global: &GovernanceConfig,
) -> (u32, u32) {
    let thresholds: Map<u32, CategoryThreshold> = env
        .storage()
        .instance()
        .get(&StorageKey::CategoryThresholds)
        .unwrap_or_else(|| Map::new(env));

    let key = category_to_key(category);
    if let Some(t) = thresholds.get(key) {
        let quorum = if t.quorum_bps > 0 {
            t.quorum_bps
        } else {
            global.quorum_threshold
        };
        let approval = if t.supermajority_bps > 0 {
            t.supermajority_bps
        } else {
            global.approval_threshold
        };
        (quorum, approval)
    } else {
        (global.quorum_threshold, global.approval_threshold)
    }
}

/// Admin-callable: store per-category quorum and supermajority overrides.
pub fn set_category_thresholds(
    env: &Env,
    admin: &Address,
    category: ProposalCategory,
    threshold: CategoryThreshold,
) -> Result<(), GovernanceError> {
    require_admin(env, admin)?;
    if threshold.quorum_bps > 10_000 || threshold.supermajority_bps > 10_000 {
        return Err(GovernanceError::InvalidGovernanceConfig);
    }
    let mut thresholds: Map<u32, CategoryThreshold> = env
        .storage()
        .instance()
        .get(&StorageKey::CategoryThresholds)
        .unwrap_or_else(|| Map::new(env));
    thresholds.set(category_to_key(&category), threshold);
    env.storage()
        .instance()
        .set(&StorageKey::CategoryThresholds, &thresholds);
    Ok(())
}

/// Read per-category thresholds for a given category (returns default if not set).
pub fn get_category_threshold(env: &Env, category: &ProposalCategory) -> Option<CategoryThreshold> {
    let thresholds: Map<u32, CategoryThreshold> = env
        .storage()
        .instance()
        .get(&StorageKey::CategoryThresholds)
        .unwrap_or_else(|| Map::new(env));
    thresholds.get(category_to_key(category))
}

fn category_to_key(category: &ProposalCategory) -> u32 {
    match category {
        ProposalCategory::ParameterChange => 0,
        ProposalCategory::ContractUpgrade => 1,
        ProposalCategory::TreasuryTransfer => 2,
        ProposalCategory::General => 3,
    }
}

// ── #837: isqrt overflow-safety tests ─────────────────────────────────────────

#[cfg(test)]
mod isqrt_tests {
    use super::*;

    #[test]
    fn isqrt_of_u128_max_does_not_panic() {
        // Regression test for #837: seeding Newton's method with `x + 1`
        // overflowed at the top of the range. This must run cleanly (no
        // panic in debug, no silent wraparound in release) and produce the
        // exact expected result: floor(sqrt(2^128 - 1)) == 2^64 - 1.
        assert_eq!(isqrt_u128(u128::MAX), 18_446_744_073_709_551_615);
    }

    #[test]
    fn isqrt_of_i128_max_does_not_panic() {
        // A whale's token balance can approach i128::MAX; the public,
        // i128-typed entry point must handle that without overflowing.
        let result = isqrt(i128::MAX);
        assert!(result > 0);
        assert!(result * result <= i128::MAX);
    }

    #[test]
    fn isqrt_known_values() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(-5), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(10), 3);
        assert_eq!(isqrt(99), 9);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(101), 10);
        assert_eq!(isqrt_u128(0), 0);
        assert_eq!(isqrt_u128(u128::MAX - 1), 18_446_744_073_709_551_615);
    }
}

#[cfg(test)]
mod isqrt_proptests {
    use super::isqrt_u128;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 4096, ..ProptestConfig::default() })]

        /// isqrt(n)^2 must never exceed n, for any n across the full u128 range.
        #[test]
        fn isqrt_squared_never_exceeds_n(n in any::<u128>()) {
            let r = isqrt_u128(n);
            let squared = r.checked_mul(r);
            prop_assert!(squared.is_some(), "isqrt({n}) = {r} but r*r overflowed u128");
            prop_assert!(squared.unwrap() <= n, "isqrt({n}) = {r} but {r}*{r} > {n}");
        }

        /// isqrt(n) must be the *floor* of the true square root: the next
        /// integer up should overshoot (when that check itself doesn't
        /// overflow, e.g. when n is u128::MAX).
        #[test]
        fn isqrt_is_the_floor_not_an_underestimate(n in any::<u128>()) {
            let r = isqrt_u128(n);
            if let Some(next_squared) = (r + 1).checked_mul(r + 1) {
                prop_assert!(next_squared > n, "isqrt({n}) = {r} but ({r}+1)^2 <= {n}");
            }
        }
    }
}
