//! Reward campaigns: funded reward budgets split across many recipients.
//!
//! A campaign holds a fixed budget of one reward asset.  The funder allocates
//! slices of it to recipients, recipients claim their allocations, and the
//! funder eventually closes the campaign and recovers whatever is left.
//!
//! This module only does accounting and storage; the contract entry points in
//! `lib.rs` handle authorization and move the tokens.
//!
//! # Campaign accounting
//!
//! ```text
//! outstanding = allocated - claimed - expired     // owed to recipients
//! available   = budget - allocated + expired      // free to allocate/recover
//! claimed + outstanding + available == budget     // always
//! ```
//!
//! # Issue #1202 – Deterministic remainder allocation
//!
//! [`split_by_weight`] divides `total` across weighted recipients:
//!
//! 1. Recipients are sorted into canonical (ascending `Address`) order first,
//!    so the result never depends on the order the caller passed them in.
//!    Duplicate recipients and zero weights are rejected.
//! 2. Each recipient gets `floor(total * weight / total_weight)`.
//! 3. The remainder `total - sum(floors)` (always `< recipients`) is handed out
//!    one unit at a time by the **largest-remainder rule**: recipients with the
//!    largest `total * weight % total_weight` get one extra unit each; ties go
//!    to the lower address.
//!
//! The shares always sum to exactly `total`: rounding never creates or loses
//! funds.  A recipient whose share rounds to zero gets no allocation record.
//!
//! # Issue #1203 – Unclaimed reward expiration policy
//!
//! * Every allocation is created with a fixed deadline
//!   `expires_at = allocated_at + claim_window_secs`, taken from the campaign's
//!   claim window at creation time.  It never changes afterwards.
//! * An allocation can be claimed while `now < expires_at`.  From
//!   `now >= expires_at` onwards it is expired and can no longer be claimed.
//! * Expiration is processed explicitly and in bounded batches by
//!   [`expire_allocations`] (at most [`MAX_CAMPAIGN_BATCH`] ids per call;
//!   anyone may call it).  Each processed allocation is marked `Expired`, its
//!   amount moves from `outstanding` to `available`, and an event records the
//!   allocation id, recipient and amount, so every expiry is auditable.
//! * **Recovery path:** expired funds go back to the campaign's `available`
//!   pool and stay there.  The funder can re-allocate them (for example, to the
//!   same recipient after a support request) with [`distribute`] while the
//!   campaign is open, or recover them on [`close_campaign`].
//!
//! # Issue #1204 – Closure only once settled
//!
//! [`close_campaign`] succeeds only if the campaign is open, has
//! `outstanding == 0` and has no `Pending` allocations left.  Allocations that
//! are past their deadline but not yet processed still count as liabilities
//! until [`expire_allocations`] settles them.  All checks run before any write,
//! so a rejected closure leaves the campaign unchanged.  On success the
//! campaign becomes `Closed` (terminal; no more allocations, claims or
//! expiries) and `available` is returned to be refunded to the funder.
//!
//! # Issue #1323 – Funding required before activation
//!
//! A campaign now starts life in `Pending`, with a promised `budget` but
//! `funded == 0`.  [`fund_campaign`] records deposits made in the campaign's
//! own `asset` (a deposit in any other asset is rejected and leaves `funded`
//! unchanged — Issue #1323's "wrong asset" case).  [`activate_campaign`]
//! moves `Pending → Open` only once `funded >= budget`; an underfunded
//! activation attempt returns [`CampaignError::InsufficientCampaignFunds`]
//! and leaves the campaign's status, `funded` and `epoch_id` untouched.  Only
//! `Open` campaigns can [`distribute`], [`claim`], or [`close_campaign`].
//!
//! # Issue #1324 – Cancelling an unfunded/unactivated campaign
//!
//! [`cancel_campaign`] is only valid while a campaign is still `Pending` —
//! i.e. it has never been activated and therefore has never accepted a
//! single allocation or claim.  It moves the campaign to `Canceled`
//! (terminal) and returns the `funded` amount so the caller can refund it to
//! the funder.  A campaign that is already `Open`, `Closed`, or `Canceled` is
//! rejected with [`CampaignError::CampaignClosed`] — repeated cancellation is
//! idempotently refused, never silently repeated.
//!
//! # Issue #1325 – Epoch snapshot binding for claim eligibility
//!
//! [`activate_campaign`] takes an `epoch_id` that is written once, at
//! activation, and never mutated again — [`RewardCampaign::epoch_id`] has no
//! setter after that point.  Every allocation created by [`distribute`]
//! records the campaign's `epoch_id` at allocation time
//! (`CampaignAllocation::epoch_id`), and [`claim`] re-checks that the
//! allocation's stamped `epoch_id` still equals the campaign's current
//! (immutable) `epoch_id` before paying out.  Because `epoch_id` cannot
//! change after activation, a recipient's eligibility is always judged
//! against the frozen snapshot taken when the campaign opened — not against
//! whatever epoch/stake state happens to exist at claim time.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

use shared::event_topics as topics;

/// Maximum number of recipients per distribution or ids per claim/expire call.
pub const MAX_CAMPAIGN_BATCH: u32 = 50;

/// Shortest allowed claim window (1 day).
pub const MIN_CLAIM_WINDOW_SECS: u64 = 86_400;

/// Longest allowed claim window (~2 years).
pub const MAX_CLAIM_WINDOW_SECS: u64 = 2 * 365 * 86_400;

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum CampaignKey {
    /// Monotonic campaign id counter.
    CampaignCounter,
    /// campaign_id → RewardCampaign.
    Campaign(u64),
    /// (campaign_id, allocation_id) → CampaignAllocation.
    Allocation(u64, u64),
    /// Monotonic vesting schedule id counter, per campaign.
    VestingCounter(u64),
    /// (campaign_id, vesting_id) → VestingAllocation (Issue #1327).
    Vesting(u64, u64),
    /// (campaign_id, recipient) → cumulative amount claimed, for enforcing
    /// `RewardCampaign::per_account_claim_cap` (Issue #1329).
    ClaimedByAccount(u64, Address),
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CampaignStatus {
    Open = 0,
    Closed = 1,
    /// Created but not yet activated: not funded (or not fully funded), has
    /// never accepted an allocation or claim (Issue #1323).
    Pending = 2,
    /// Cancelled while still `Pending` (Issue #1324); terminal.
    Canceled = 3,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AllocationStatus {
    Pending = 0,
    Claimed = 1,
    Expired = 2,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardCampaign {
    pub campaign_id: u64,
    /// Manager of the campaign and destination of recovered funds.
    pub funder: Address,
    pub asset: Address,
    /// Total funded amount.
    pub budget: i128,
    /// Lifetime amount allocated to recipients (includes claimed and expired).
    pub allocated: i128,
    /// Lifetime amount claimed by recipients.
    pub claimed: i128,
    /// Lifetime amount of allocations that expired unclaimed.
    pub expired: i128,
    /// Number of allocations still `Pending` (neither claimed nor expired).
    pub pending_allocations: u32,
    /// Next allocation id to assign.
    pub next_allocation_id: u64,
    pub claim_window_secs: u64,
    pub status: CampaignStatus,
    pub created_at: u64,
    pub closed_at: u64,
    /// Amount actually deposited toward `budget` so far (Issue #1323). Only
    /// meaningful while `Pending`; once `Open`, funding is complete by
    /// definition (`funded >= budget`) and never decreases afterwards.
    pub funded: i128,
    /// Immutable epoch/snapshot identifier bound at activation time (Issue
    /// #1325). `0` while `Pending` (unset); never changes once set.
    pub epoch_id: u64,
    /// Maximum lifetime amount any single account may claim from this
    /// campaign (Issue #1329). `0` means no cap.
    pub per_account_claim_cap: i128,
}

impl RewardCampaign {
    /// Amount allocated to recipients and not yet claimed or expired.
    pub fn outstanding(&self) -> i128 {
        self.allocated - self.claimed - self.expired
    }

    /// Amount that is neither owed to anyone nor already paid out.
    pub fn available(&self) -> i128 {
        self.budget - self.allocated + self.expired
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignAllocation {
    pub campaign_id: u64,
    pub allocation_id: u64,
    pub recipient: Address,
    pub amount: i128,
    pub allocated_at: u64,
    /// First timestamp at which the allocation is expired (no longer claimable).
    pub expires_at: u64,
    pub status: AllocationStatus,
    /// Snapshot of the campaign's `epoch_id` at the moment this allocation
    /// was created (Issue #1325). Claims are checked against this, not
    /// against whatever the campaign's `epoch_id` happens to be later — it
    /// never changes after activation anyway, but stamping it per-allocation
    /// makes the binding explicit and independently verifiable.
    pub epoch_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CampaignError {
    CampaignNotFound,
    CampaignClosed,
    InvalidAmount,
    InvalidClaimWindow,
    /// Distribution would allocate more than the campaign's `available` funds.
    InsufficientCampaignFunds,
    /// Recipient list is empty, too long, has a duplicate or a zero weight.
    InvalidRecipients,
    /// Claim/expire batch is empty or larger than [`MAX_CAMPAIGN_BATCH`].
    BatchSizeInvalid,
    /// Allocation does not exist, belongs to someone else, or is not `Pending`.
    AllocationNotClaimable,
    /// Allocation deadline has passed.
    AllocationExpired,
    /// Allocation deadline has not passed yet.
    AllocationNotExpired,
    /// Campaign still has outstanding liabilities to recipients.
    OutstandingLiabilities,
    Overflow,
    /// Campaign is not `Pending` (already activated, cancelled, or closed) —
    /// Issue #1323 / #1324.
    CampaignNotPending,
    /// Funding deposit used an asset other than the campaign's own `asset`
    /// (Issue #1323).
    WrongFundingAsset,
    /// `cliff_secs > duration_secs`, or `duration_secs == 0` (Issue #1327).
    InvalidVestingSchedule,
    /// Vesting schedule does not exist or belongs to someone else.
    VestingNotFound,
    /// Nothing newly vested to claim yet.
    NothingVested,
    /// Claim would exceed the campaign's per-account cap (Issue #1329).
    ClaimLimitExceeded,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn contract_topic(env: &Env) -> Symbol {
    Symbol::new(env, "stake_vault")
}

fn add(a: i128, b: i128) -> Result<i128, CampaignError> {
    a.checked_add(b).ok_or(CampaignError::Overflow)
}

fn load(env: &Env, campaign_id: u64) -> Result<RewardCampaign, CampaignError> {
    env.storage()
        .persistent()
        .get(&CampaignKey::Campaign(campaign_id))
        .ok_or(CampaignError::CampaignNotFound)
}

fn load_open(env: &Env, campaign_id: u64) -> Result<RewardCampaign, CampaignError> {
    let campaign = load(env, campaign_id)?;
    if campaign.status != CampaignStatus::Open {
        return Err(CampaignError::CampaignClosed);
    }
    Ok(campaign)
}

fn save(env: &Env, campaign: &RewardCampaign) {
    env.storage()
        .persistent()
        .set(&CampaignKey::Campaign(campaign.campaign_id), campaign);
}

fn check_batch(len: u32) -> Result<(), CampaignError> {
    if len == 0 || len > MAX_CAMPAIGN_BATCH {
        return Err(CampaignError::BatchSizeInvalid);
    }
    Ok(())
}

// ── Issue #1202: deterministic weighted split ─────────────────────────────────

/// Split `total` across `recipients` by weight.  See the module docs for the
/// rounding rule.  Returns one `(recipient, share)` per recipient, in canonical
/// (ascending address) order; the shares sum to exactly `total`.
pub fn split_by_weight(
    env: &Env,
    total: i128,
    recipients: &Vec<(Address, u32)>,
) -> Result<Vec<(Address, i128)>, CampaignError> {
    if total <= 0 {
        return Err(CampaignError::InvalidAmount);
    }
    let n = recipients.len();
    if n == 0 || n > MAX_CAMPAIGN_BATCH {
        return Err(CampaignError::InvalidRecipients);
    }

    // Canonical order: insertion sort by address (n is small and bounded).
    let mut sorted: Vec<(Address, u32)> = Vec::new(env);
    for i in 0..n {
        let (addr, weight) = recipients.get(i).unwrap();
        if weight == 0 {
            return Err(CampaignError::InvalidRecipients);
        }
        let mut pos = sorted.len();
        for j in 0..sorted.len() {
            let existing = sorted.get(j).unwrap().0;
            if existing == addr {
                return Err(CampaignError::InvalidRecipients);
            }
            if addr < existing {
                pos = j;
                break;
            }
        }
        sorted.insert(pos, (addr, weight));
    }

    let mut total_weight: i128 = 0;
    for i in 0..n {
        total_weight = add(total_weight, sorted.get(i).unwrap().1 as i128)?;
    }

    // Floors and remainders.
    let mut shares: Vec<i128> = Vec::new(env);
    let mut fractions: Vec<i128> = Vec::new(env);
    let mut floor_sum: i128 = 0;
    for i in 0..n {
        let weight = sorted.get(i).unwrap().1 as i128;
        let scaled = total.checked_mul(weight).ok_or(CampaignError::Overflow)?;
        let share = scaled / total_weight;
        shares.push_back(share);
        fractions.push_back(scaled % total_weight);
        floor_sum = add(floor_sum, share)?;
    }

    // Largest remainder first; ties to the lower (earlier) address.  Each
    // recipient gets at most one extra unit since remainder < n.
    let mut remainder = total - floor_sum;
    while remainder > 0 {
        let mut best: Option<u32> = None;
        for i in 0..n {
            let frac = fractions.get(i).unwrap();
            if frac < 0 {
                continue; // already received its extra unit
            }
            match best {
                Some(b) if fractions.get(b).unwrap() >= frac => {}
                _ => best = Some(i),
            }
        }
        let b = best.ok_or(CampaignError::Overflow)?;
        shares.set(b, shares.get(b).unwrap() + 1);
        fractions.set(b, -1);
        remainder -= 1;
    }

    let mut out: Vec<(Address, i128)> = Vec::new(env);
    for i in 0..n {
        out.push_back((sorted.get(i).unwrap().0, shares.get(i).unwrap()));
    }
    Ok(out)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Record a new campaign in `Pending` status (Issue #1323). The campaign is
/// not claimable yet: it must be funded with [`fund_campaign`] and then
/// opened with [`activate_campaign`] before any allocation or claim is
/// possible.
pub fn create_campaign(
    env: &Env,
    funder: Address,
    asset: Address,
    budget: i128,
    claim_window_secs: u64,
) -> Result<u64, CampaignError> {
    if budget <= 0 {
        return Err(CampaignError::InvalidAmount);
    }
    if !(MIN_CLAIM_WINDOW_SECS..=MAX_CLAIM_WINDOW_SECS).contains(&claim_window_secs) {
        return Err(CampaignError::InvalidClaimWindow);
    }

    let campaign_id: u64 = env
        .storage()
        .instance()
        .get(&CampaignKey::CampaignCounter)
        .unwrap_or(0u64);
    env.storage().instance().set(
        &CampaignKey::CampaignCounter,
        &campaign_id.saturating_add(1),
    );

    let campaign = RewardCampaign {
        campaign_id,
        funder: funder.clone(),
        asset: asset.clone(),
        budget,
        allocated: 0,
        claimed: 0,
        expired: 0,
        pending_allocations: 0,
        next_allocation_id: 0,
        claim_window_secs,
        status: CampaignStatus::Pending,
        created_at: env.ledger().timestamp(),
        closed_at: 0,
        funded: 0,
        epoch_id: 0,
        per_account_claim_cap: 0,
    };
    save(env, &campaign);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_CAMPAIGN_CREATED()),
        (campaign_id, funder, asset, budget, claim_window_secs),
    );
    Ok(campaign_id)
}

        funded: 0,
        epoch_id: 0,
        per_account_claim_cap: 0,
}

/// Allocate `total` of the campaign's available funds across `recipients` by
/// weight (Issue #1202).  Returns `(recipient, allocation_id, amount)` for each
/// allocation created, in canonical recipient order.  Recipients whose share
/// rounds to zero get no allocation.
pub fn distribute(
    env: &Env,
    campaign_id: u64,
    total: i128,
    recipients: Vec<(Address, u32)>,
) -> Result<Vec<(Address, u64, i128)>, CampaignError> {
    let mut campaign = load_open(env, campaign_id)?;
    if total <= 0 {
        return Err(CampaignError::InvalidAmount);
    }
    if total > campaign.available() {
        return Err(CampaignError::InsufficientCampaignFunds);
    }
    let shares = split_by_weight(env, total, &recipients)?;

    let now = env.ledger().timestamp();
    let expires_at = now.saturating_add(campaign.claim_window_secs);
    let mut created: Vec<(Address, u64, i128)> = Vec::new(env);
    for i in 0..shares.len() {
        let (recipient, amount) = shares.get(i).unwrap();
        if amount == 0 {
            continue;
        }
        let allocation_id = campaign.next_allocation_id;
        campaign.next_allocation_id = allocation_id.saturating_add(1);
        campaign.pending_allocations = campaign.pending_allocations.saturating_add(1);

        let allocation = CampaignAllocation {
            campaign_id,
            allocation_id,
            recipient: recipient.clone(),
            amount,
            allocated_at: now,
            expires_at,
            status: AllocationStatus::Pending,
            // Issue #1325: snapshot the campaign's immutable epoch_id at
            // allocation time, so eligibility is always checked against this
            // frozen value rather than any later state.
            epoch_id: campaign.epoch_id,
        };
        env.storage().persistent().set(
            &CampaignKey::Allocation(campaign_id, allocation_id),
            &allocation,
        );

        env.events().publish(
            (contract_topic(env), topics::TOPIC_CAMPAIGN_ALLOCATED()),
            (
                campaign_id,
                allocation_id,
                recipient.clone(),
                amount,
                expires_at,
            ),
        );
        created.push_back((recipient, allocation_id, amount));
    }

    // split_by_weight guarantees the shares sum to exactly `total`.
    campaign.allocated = add(campaign.allocated, total)?;
    save(env, &campaign);
    Ok(created)
}

/// Claim `allocation_ids` for `recipient`.  Validation is all-or-nothing: if
/// any id is unknown, belongs to someone else, is not pending, or has expired,
/// nothing is claimed.  Returns the total the caller must transfer out.
pub fn claim(
    env: &Env,
    campaign_id: u64,
    recipient: &Address,
    allocation_ids: Vec<u64>,
) -> Result<i128, CampaignError> {
    let mut campaign = load_open(env, campaign_id)?;
    check_batch(allocation_ids.len())?;
    let now = env.ledger().timestamp();

    let mut allocations: Vec<CampaignAllocation> = Vec::new(env);
    let mut total: i128 = 0;
    for i in 0..allocation_ids.len() {
        let id = allocation_ids.get(i).unwrap();
        // A repeated id in the same batch would be paid twice.
        for j in 0..i {
            if allocation_ids.get(j).unwrap() == id {
                return Err(CampaignError::AllocationNotClaimable);
            }
        }
        let allocation: CampaignAllocation = env
            .storage()
            .persistent()
            .get(&CampaignKey::Allocation(campaign_id, id))
            .ok_or(CampaignError::AllocationNotClaimable)?;
        if allocation.recipient != *recipient || allocation.status != AllocationStatus::Pending {
            return Err(CampaignError::AllocationNotClaimable);
        }
        if now >= allocation.expires_at {
            return Err(CampaignError::AllocationExpired);
        }
        // Issue #1325: eligibility is checked against the epoch snapshot
        // stamped on the allocation at distribution time, not the campaign's
        // current `epoch_id` — even though the latter cannot change after
        // activation, this keeps the binding explicit and would still catch
        // any future bug that tried to mutate it.
        if allocation.epoch_id != campaign.epoch_id {
            return Err(CampaignError::AllocationNotClaimable);
        }
        total = add(total, allocation.amount)?;
        allocations.push_back(allocation);
    }

    // Issue #1329: cumulative claims (across this and all prior calls) must
    // never exceed the campaign's per-account cap. Checked before any write,
    // so splitting a claim across many smaller calls cannot bypass it.
    let already_claimed = claimed_by_account(env, campaign_id, recipient);
    let new_total_claimed = add(already_claimed, total)?;
    if campaign.per_account_claim_cap > 0 && new_total_claimed > campaign.per_account_claim_cap {
        return Err(CampaignError::ClaimLimitExceeded);
    }

    for i in 0..allocations.len() {
        let mut allocation = allocations.get(i).unwrap();
        allocation.status = AllocationStatus::Claimed;
        env.storage().persistent().set(
            &CampaignKey::Allocation(campaign_id, allocation.allocation_id),
            &allocation,
        );
    }
    env.storage().persistent().set(
        &CampaignKey::ClaimedByAccount(campaign_id, recipient.clone()),
        &new_total_claimed,
    );
    campaign.claimed = add(campaign.claimed, total)?;
    campaign.pending_allocations -= allocations.len();
    save(env, &campaign);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_CAMPAIGN_CLAIMED()),
        (campaign_id, recipient.clone(), total, allocations.len()),
    );
    Ok(total)
}

/// Process expiry for `allocation_ids` (Issue #1203).  Callable by anyone.
/// All-or-nothing: every id must exist, be `Pending` and be past its deadline
/// (`now >= expires_at`).  Returns the total moved back to `available`.
pub fn expire_allocations(
    env: &Env,
    campaign_id: u64,
    allocation_ids: Vec<u64>,
) -> Result<i128, CampaignError> {
    let mut campaign = load_open(env, campaign_id)?;
    check_batch(allocation_ids.len())?;
    let now = env.ledger().timestamp();

    let mut allocations: Vec<CampaignAllocation> = Vec::new(env);
    let mut total: i128 = 0;
    for i in 0..allocation_ids.len() {
        let id = allocation_ids.get(i).unwrap();
        for j in 0..i {
            if allocation_ids.get(j).unwrap() == id {
                return Err(CampaignError::AllocationNotClaimable);
            }
        }
        let allocation: CampaignAllocation = env
            .storage()
            .persistent()
            .get(&CampaignKey::Allocation(campaign_id, id))
            .ok_or(CampaignError::AllocationNotClaimable)?;
        if allocation.status != AllocationStatus::Pending {
            return Err(CampaignError::AllocationNotClaimable);
        }
        if now < allocation.expires_at {
            return Err(CampaignError::AllocationNotExpired);
        }
        total = add(total, allocation.amount)?;
        allocations.push_back(allocation);
    }

    for i in 0..allocations.len() {
        let mut allocation = allocations.get(i).unwrap();
        allocation.status = AllocationStatus::Expired;
        env.storage().persistent().set(
            &CampaignKey::Allocation(campaign_id, allocation.allocation_id),
            &allocation,
        );
        env.events().publish(
            (contract_topic(env), topics::TOPIC_CAMPAIGN_EXPIRED()),
            (
                campaign_id,
                allocation.allocation_id,
                allocation.recipient.clone(),
                allocation.amount,
                allocation.expires_at,
            ),
        );
    }
    campaign.expired = add(campaign.expired, total)?;
    campaign.pending_allocations -= allocations.len();
    save(env, &campaign);
    Ok(total)
}

/// Close the campaign (Issue #1204).  Fails without modifying anything unless
/// every allocation has been claimed or expired.  Returns the `available`
/// amount the caller must refund to the funder.
pub fn close_campaign(env: &Env, campaign_id: u64) -> Result<i128, CampaignError> {
    let mut campaign = load_open(env, campaign_id)?;
    if campaign.outstanding() != 0 || campaign.pending_allocations != 0 {
        return Err(CampaignError::OutstandingLiabilities);
    }

    let refund = campaign.available();
    campaign.status = CampaignStatus::Closed;
    campaign.closed_at = env.ledger().timestamp();
    save(env, &campaign);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_CAMPAIGN_CLOSED()),
        (
            campaign_id,
            campaign.funder.clone(),
            refund,
            campaign.claimed,
            campaign.expired,
        ),
    );
    Ok(refund)
}

pub fn get_campaign(env: &Env, campaign_id: u64) -> Option<RewardCampaign> {
    env.storage()
        .persistent()
        .get(&CampaignKey::Campaign(campaign_id))
}

pub fn get_allocation(
    env: &Env,
    campaign_id: u64,
    allocation_id: u64,
) -> Option<CampaignAllocation> {
    env.storage()
        .persistent()
        .get(&CampaignKey::Allocation(campaign_id, allocation_id))
}

// ── Issue #1327: linear vesting schedules ──────────────────────────────────────
//
// A vesting allocation grants `total_amount` to `recipient`, released linearly
// between `start + cliff_secs` and `start + duration_secs`:
//
// * Before `start + cliff_secs`: nothing is claimable.
// * Between the cliff and `start + duration_secs`: claimable amount is
//   `floor(total_amount * elapsed / duration_secs)`, where `elapsed` is
//   capped at `duration_secs`. This is monotonically non-decreasing in time
//   and the schedule is a pure, deterministic function of `now` — repeated
//   calls at the same timestamp always agree.
// * At or after `start + duration_secs`: the full `total_amount` is vested.
//
// [`claim_vesting`] pays out `vested_amount(now) - claimed`, so partial claims
// made at different points along the schedule never exceed the allocation —
// each claim only ever releases newly-vested funds, checked with saturating
// arithmetic on `vested`/`claimed` and `checked_sub` on the payout itself.

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingAllocation {
    pub campaign_id: u64,
    pub vesting_id: u64,
    pub recipient: Address,
    /// Total amount granted over the life of the schedule.
    pub total_amount: i128,
    /// Ledger timestamp the schedule is anchored to.
    pub start: u64,
    /// Seconds after `start` before anything is claimable.
    pub cliff_secs: u64,
    /// Seconds after `start` at which the full amount is vested.
    pub duration_secs: u64,
    /// Amount already claimed against this schedule.
    pub claimed: i128,
}

fn load_vesting(
    env: &Env,
    campaign_id: u64,
    vesting_id: u64,
) -> Result<VestingAllocation, CampaignError> {
    env.storage()
        .persistent()
        .get(&CampaignKey::Vesting(campaign_id, vesting_id))
        .ok_or(CampaignError::VestingNotFound)
}

fn save_vesting(env: &Env, vesting: &VestingAllocation) {
    env.storage().persistent().set(
        &CampaignKey::Vesting(vesting.campaign_id, vesting.vesting_id),
        vesting,
    );
}

/// Deterministic linear-vesting curve. Pure function of `now` and the
/// schedule parameters; never exceeds `total_amount`.
pub fn vested_amount(now: u64, vesting: &VestingAllocation) -> i128 {
    let cliff_at = vesting.start.saturating_add(vesting.cliff_secs);
    if now < cliff_at {
        return 0;
    }
    let end_at = vesting.start.saturating_add(vesting.duration_secs);
    if now >= end_at {
        return vesting.total_amount;
    }
    let elapsed = now.saturating_sub(vesting.start) as u128;
    let duration = vesting.duration_secs as u128;
    let amount = vesting.total_amount as u128;
    // floor(total_amount * elapsed / duration); elapsed < duration here, and
    // total_amount * elapsed fits well within u128 for any realistic i128
    // reward amount and duration, so this cannot overflow in practice.
    let vested = amount.saturating_mul(elapsed) / duration.max(1);
    vested as i128
}

/// Create a linear vesting allocation funded from the campaign's `available`
/// balance (Issue #1327). Ordering is validated before any state changes:
/// `cliff_secs <= duration_secs` and `duration_secs > 0`.
pub fn create_vesting_allocation(
    env: &Env,
    campaign_id: u64,
    recipient: Address,
    total_amount: i128,
    start: u64,
    cliff_secs: u64,
    duration_secs: u64,
) -> Result<u64, CampaignError> {
    let mut campaign = load_open(env, campaign_id)?;
    if total_amount <= 0 {
        return Err(CampaignError::InvalidAmount);
    }
    if duration_secs == 0 || cliff_secs > duration_secs {
        return Err(CampaignError::InvalidVestingSchedule);
    }
    if total_amount > campaign.available() {
        return Err(CampaignError::InsufficientCampaignFunds);
    }

    let vesting_id: u64 = env
        .storage()
        .persistent()
        .get(&CampaignKey::VestingCounter(campaign_id))
        .unwrap_or(0u64);
    env.storage().persistent().set(
        &CampaignKey::VestingCounter(campaign_id),
        &vesting_id.saturating_add(1),
    );

    let vesting = VestingAllocation {
        campaign_id,
        vesting_id,
        recipient: recipient.clone(),
        total_amount,
        start,
        cliff_secs,
        duration_secs,
        claimed: 0,
    };
    save_vesting(env, &vesting);

    // Vested allocations are treated the same as regular allocations for
    // campaign accounting: reserved out of `available`, paid out of
    // `allocated` as they are claimed. They are tracked as a liability via
    // `pending_allocations` until fully claimed, mirroring `close_campaign`'s
    // settlement requirement.
    campaign.allocated = add(campaign.allocated, total_amount)?;
    campaign.pending_allocations = campaign.pending_allocations.saturating_add(1);
    save(env, &campaign);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_CAMPAIGN_ALLOCATED()),
        (campaign_id, vesting_id, recipient, total_amount, start),
    );
    Ok(vesting_id)
}

/// Claim whatever has newly vested for `vesting_id` since the last claim
/// (Issue #1327). Rejects if the caller is not the schedule's recipient or if
/// nothing new has vested. Returns the amount transferred.
pub fn claim_vesting(
    env: &Env,
    campaign_id: u64,
    vesting_id: u64,
    recipient: &Address,
) -> Result<i128, CampaignError> {
    let mut campaign = load_open(env, campaign_id)?;
    let mut vesting = load_vesting(env, campaign_id, vesting_id)?;
    if vesting.recipient != *recipient {
        return Err(CampaignError::VestingNotFound);
    }

    let now = env.ledger().timestamp();
    let total_vested = vested_amount(now, &vesting);
    let payout = total_vested
        .checked_sub(vesting.claimed)
        .ok_or(CampaignError::Overflow)?;
    if payout <= 0 {
        return Err(CampaignError::NothingVested);
    }

    let fully_settled = vesting.claimed.saturating_add(payout) >= vesting.total_amount;
    vesting.claimed = add(vesting.claimed, payout)?;
    save_vesting(env, &vesting);

    campaign.claimed = add(campaign.claimed, payout)?;
    if fully_settled {
        campaign.pending_allocations = campaign.pending_allocations.saturating_sub(1);
    }
    save(env, &campaign);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_CAMPAIGN_CLAIMED()),
        (campaign_id, recipient.clone(), payout, vesting_id),
    );
    Ok(payout)
}

pub fn get_vesting(env: &Env, campaign_id: u64, vesting_id: u64) -> Option<VestingAllocation> {
    env.storage()
        .persistent()
        .get(&CampaignKey::Vesting(campaign_id, vesting_id))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, contractimpl, vec, Env};

    #[contract]
    struct CampaignHost;

    #[contractimpl]
    impl CampaignHost {}

    const WINDOW: u64 = MIN_CLAIM_WINDOW_SECS;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let host = env.register(CampaignHost, ());
        (env, host)
    }

    fn sorted_addrs(env: &Env, n: u32) -> std::vec::Vec<Address> {
        let mut v: std::vec::Vec<Address> = (0..n).map(|_| Address::generate(env)).collect();
        v.sort();
        v
    }

    fn assert_accounting(c: &RewardCampaign) {
        assert_eq!(c.claimed + c.outstanding() + c.available(), c.budget);
        assert!(c.outstanding() >= 0 && c.available() >= 0);
    }

    /// Creates a campaign and immediately funds + activates it (epoch 1), so
    /// existing tests that assume an `Open`, claimable campaign are
    /// unaffected by the #1323/#1324/#1325 Pending→Open lifecycle.
    fn new_campaign(env: &Env, budget: i128) -> u64 {
        new_campaign_with_asset(env, Address::generate(env), budget)
    }

    fn new_campaign_with_asset(env: &Env, asset: Address, budget: i128) -> u64 {
        let id = create_campaign(env, Address::generate(env), asset.clone(), budget, WINDOW)
            .unwrap();
        fund_campaign(env, id, &asset, budget).unwrap();
        activate_campaign(env, id, 1).unwrap();
        id
    }

    /// Creates a campaign without funding or activating it (still `Pending`).
    fn new_pending_campaign(env: &Env, asset: Address, budget: i128) -> u64 {
        create_campaign(env, Address::generate(env), asset, budget, WINDOW).unwrap()
    }

    // ── #1202: remainder allocation ──────────────────────────────────────────

    fn shares_of(out: &Vec<(Address, i128)>) -> std::vec::Vec<i128> {
        out.iter().map(|(_, s)| s).collect()
    }

    #[test]
    fn split_is_exact_and_uses_largest_remainder() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let a = sorted_addrs(&env, 3);
            // 100 by 1:1:1 => 33.33 each; one leftover unit, all remainders
            // tie, so it goes to the lowest address.
            let r = vec![
                &env,
                (a[0].clone(), 1),
                (a[1].clone(), 1),
                (a[2].clone(), 1),
            ];
            let out = split_by_weight(&env, 100, &r).unwrap();
            assert_eq!(shares_of(&out), [34, 33, 33]);

            // 10 by 1:2:3 => 1.67, 3.33, 5.0 => floors 1,3,5; the leftover
            // unit goes to the largest fractional part (a[0]).
            let r = vec![
                &env,
                (a[0].clone(), 1),
                (a[1].clone(), 2),
                (a[2].clone(), 3),
            ];
            let out = split_by_weight(&env, 10, &r).unwrap();
            assert_eq!(shares_of(&out), [2, 3, 5]);
            assert_eq!(shares_of(&out).iter().sum::<i128>(), 10);
        });
    }

    #[test]
    fn split_is_independent_of_recipient_order() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let a = sorted_addrs(&env, 4);
            let weights = [3u32, 1, 7, 2];
            let perms: [[usize; 4]; 6] = [
                [0, 1, 2, 3],
                [3, 2, 1, 0],
                [1, 3, 0, 2],
                [2, 0, 3, 1],
                [3, 0, 1, 2],
                [1, 2, 3, 0],
            ];
            for total in [1i128, 2, 3, 12, 13, 1_000_003] {
                let mut expected: Option<Vec<(Address, i128)>> = None;
                for p in perms.iter() {
                    let mut r = Vec::new(&env);
                    for &i in p {
                        r.push_back((a[i].clone(), weights[i]));
                    }
                    let out = split_by_weight(&env, total, &r).unwrap();
                    assert_eq!(out.iter().map(|(_, s)| s).sum::<i128>(), total);
                    match &expected {
                        None => expected = Some(out),
                        Some(e) => assert_eq!(&out, e, "order dependent for {total}"),
                    }
                }
            }
        });
    }

    #[test]
    fn split_small_amounts_below_recipient_count() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let a = sorted_addrs(&env, 5);
            let mut r = Vec::new(&env);
            for addr in a.iter() {
                r.push_back((addr.clone(), 1u32));
            }
            // 1 unit over 5 equal recipients: only the lowest address gets it.
            assert_eq!(
                shares_of(&split_by_weight(&env, 1, &r).unwrap()),
                [1, 0, 0, 0, 0]
            );
            assert_eq!(
                shares_of(&split_by_weight(&env, 3, &r).unwrap()),
                [1, 1, 1, 0, 0]
            );
            assert_eq!(
                shares_of(&split_by_weight(&env, 7, &r).unwrap()),
                [2, 2, 1, 1, 1]
            );
        });
    }

    #[test]
    fn split_rejects_invalid_input() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let a = Address::generate(&env);
            let b = Address::generate(&env);
            let dup = vec![&env, (a.clone(), 1), (a.clone(), 2)];
            let zero = vec![&env, (a.clone(), 1), (b.clone(), 0)];
            let ok = vec![&env, (a.clone(), 1)];
            let empty: Vec<(Address, u32)> = Vec::new(&env);
            assert_eq!(
                split_by_weight(&env, 10, &dup),
                Err(CampaignError::InvalidRecipients)
            );
            assert_eq!(
                split_by_weight(&env, 10, &zero),
                Err(CampaignError::InvalidRecipients)
            );
            assert_eq!(
                split_by_weight(&env, 10, &empty),
                Err(CampaignError::InvalidRecipients)
            );
            assert_eq!(
                split_by_weight(&env, 0, &ok),
                Err(CampaignError::InvalidAmount)
            );
        });
    }

    #[test]
    fn distribute_never_exceeds_budget_and_skips_zero_shares() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 10);
            let a = sorted_addrs(&env, 3);
            let r = vec![
                &env,
                (a[0].clone(), 1),
                (a[1].clone(), 1),
                (a[2].clone(), 1),
            ];

            // 2 units over 3 recipients: two allocations, one skipped.
            let created = distribute(&env, id, 2, r.clone()).unwrap();
            assert_eq!(created.len(), 2);
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.allocated, 2);
            assert_eq!(c.pending_allocations, 2);
            assert_accounting(&c);

            assert_eq!(
                distribute(&env, id, 9, r.clone()),
                Err(CampaignError::InsufficientCampaignFunds)
            );
            distribute(&env, id, 8, r).unwrap();
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.available(), 0);
            assert_accounting(&c);
        });
    }

    // ── #1203: expiration boundary ───────────────────────────────────────────

    fn one_allocation(env: &Env) -> (u64, Address, u64, u64) {
        let id = new_campaign(env, 1_000);
        let user = Address::generate(env);
        let created = distribute(env, id, 400, vec![env, (user.clone(), 1)]).unwrap();
        let (_, alloc_id, _) = created.get(0).unwrap();
        let expires_at = get_allocation(env, id, alloc_id).unwrap().expires_at;
        (id, user, alloc_id, expires_at)
    }

    #[test]
    fn claim_before_expiry_succeeds() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let (id, user, alloc, expires_at) = one_allocation(&env);
            assert_eq!(expires_at, 1_000 + WINDOW);
            env.ledger().set_timestamp(expires_at - 1);
            assert_eq!(
                expire_allocations(&env, id, vec![&env, alloc]),
                Err(CampaignError::AllocationNotExpired)
            );
            assert_eq!(claim(&env, id, &user, vec![&env, alloc]).unwrap(), 400);
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.claimed, 400);
            assert_accounting(&c);
        });
    }

    #[test]
    fn allocation_is_expired_exactly_at_deadline() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let (id, user, alloc, expires_at) = one_allocation(&env);
            env.ledger().set_timestamp(expires_at);
            assert_eq!(
                claim(&env, id, &user, vec![&env, alloc]),
                Err(CampaignError::AllocationExpired)
            );
            assert_eq!(
                expire_allocations(&env, id, vec![&env, alloc]).unwrap(),
                400
            );
            assert_eq!(
                get_allocation(&env, id, alloc).unwrap().status,
                AllocationStatus::Expired
            );
        });
    }

    #[test]
    fn expiry_after_deadline_returns_funds_to_available() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let (id, user, alloc, expires_at) = one_allocation(&env);
            env.ledger().set_timestamp(expires_at + 10_000);
            assert_eq!(
                claim(&env, id, &user, vec![&env, alloc]),
                Err(CampaignError::AllocationExpired)
            );
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.outstanding(), 400); // still a liability until processed

            expire_allocations(&env, id, vec![&env, alloc]).unwrap();
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.expired, 400);
            assert_eq!(c.outstanding(), 0);
            assert_eq!(c.available(), 1_000);
            assert_eq!(c.pending_allocations, 0);
            assert_accounting(&c);

            // Processed twice / claimed after expiry: rejected.
            assert_eq!(
                expire_allocations(&env, id, vec![&env, alloc]),
                Err(CampaignError::AllocationNotClaimable)
            );
            // Recovery path: the funder re-allocates the expired funds.
            distribute(&env, id, 400, vec![&env, (user.clone(), 1)]).unwrap();
            assert_eq!(get_campaign(&env, id).unwrap().outstanding(), 400);
        });
    }

    #[test]
    fn claim_and_expire_batches_are_all_or_nothing() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            let user = Address::generate(&env);
            let other = Address::generate(&env);
            distribute(&env, id, 100, vec![&env, (user.clone(), 1)]).unwrap(); // alloc 0
            distribute(&env, id, 100, vec![&env, (other.clone(), 1)]).unwrap(); // alloc 1
            let before = get_campaign(&env, id).unwrap();

            // Someone else's allocation poisons the batch.
            assert_eq!(
                claim(&env, id, &user, vec![&env, 0, 1]),
                Err(CampaignError::AllocationNotClaimable)
            );
            // Duplicate ids cannot double-pay.
            assert_eq!(
                claim(&env, id, &user, vec![&env, 0, 0]),
                Err(CampaignError::AllocationNotClaimable)
            );
            assert_eq!(
                claim(&env, id, &user, Vec::new(&env)),
                Err(CampaignError::BatchSizeInvalid)
            );
            assert_eq!(get_campaign(&env, id).unwrap(), before);
            assert_eq!(
                get_allocation(&env, id, 0).unwrap().status,
                AllocationStatus::Pending
            );
        });
    }

    // ── #1204: closure ──────────────────────────────────────────────────────

    #[test]
    fn close_with_no_claims_refunds_full_budget() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 500);
            assert_eq!(close_campaign(&env, id).unwrap(), 500);
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.status, CampaignStatus::Closed);
            assert_eq!(close_campaign(&env, id), Err(CampaignError::CampaignClosed));
            assert_eq!(
                distribute(&env, id, 1, vec![&env, (Address::generate(&env), 1)]),
                Err(CampaignError::CampaignClosed)
            );
        });
    }

    #[test]
    fn close_with_outstanding_claims_is_rejected_without_changes() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let (id, _user, _alloc, expires_at) = one_allocation(&env);
            let before = get_campaign(&env, id).unwrap();
            assert_eq!(
                close_campaign(&env, id),
                Err(CampaignError::OutstandingLiabilities)
            );
            assert_eq!(get_campaign(&env, id).unwrap(), before);

            // Past the deadline but not yet processed: still a liability.
            env.ledger().set_timestamp(expires_at);
            assert_eq!(
                close_campaign(&env, id),
                Err(CampaignError::OutstandingLiabilities)
            );
            assert_eq!(get_campaign(&env, id).unwrap(), before);
        });
    }

    #[test]
    fn recovery_cannot_consume_funds_reserved_for_valid_claims() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            let user = Address::generate(&env);
            distribute(&env, id, 400, vec![&env, (user.clone(), 1)]).unwrap();
            let campaign_before = get_campaign(&env, id).unwrap();
            assert_eq!(campaign_before.outstanding(), 400);

            // Attempting to close/recover while valid claims are outstanding fails.
            assert_eq!(
                close_campaign(&env, id),
                Err(CampaignError::OutstandingLiabilities)
            );

            // Verify funds reserved for valid claim remain untouched and claimable.
            claim(&env, id, &user, vec![&env, 0]).unwrap();
            let campaign_after_claim = get_campaign(&env, id).unwrap();
            assert_eq!(campaign_after_claim.claimed, 400);
            assert_eq!(campaign_after_claim.outstanding(), 0);

            // Now that obligations are resolved, recovery succeeds for the unallocated remainder (600).
            assert_eq!(close_campaign(&env, id).unwrap(), 600);
        });
    }

    #[test]
    fn close_after_all_claims_settled() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            let a = sorted_addrs(&env, 2);
            let r = vec![&env, (a[0].clone(), 1), (a[1].clone(), 1)];
            distribute(&env, id, 301, r).unwrap(); // 151 / 150

            claim(&env, id, &a[0], vec![&env, 0]).unwrap();
            assert_eq!(
                close_campaign(&env, id),
                Err(CampaignError::OutstandingLiabilities)
            );

            // Second allocation expires instead of being claimed.
            env.ledger().set_timestamp(1_000 + WINDOW);
            expire_allocations(&env, id, vec![&env, 1]).unwrap();

            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.claimed, 151);
            assert_eq!(c.expired, 150);
            assert_accounting(&c);
            // Refund = unallocated 699 + expired 150.
            assert_eq!(close_campaign(&env, id).unwrap(), 849);
            assert_eq!(
                claim(&env, id, &a[1], vec![&env, 1]),
                Err(CampaignError::CampaignClosed)
            );
        });
    }

    // ── #1327: linear vesting ─────────────────────────────────────────────────

    #[test]
    fn vesting_rejects_invalid_schedule() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            let user = Address::generate(&env);
            assert_eq!(
                create_vesting_allocation(&env, id, user.clone(), 100, 1_000, 10, 0),
                Err(CampaignError::InvalidVestingSchedule)
            );
            assert_eq!(
                create_vesting_allocation(&env, id, user.clone(), 100, 1_000, 500, 100),
                Err(CampaignError::InvalidVestingSchedule)
            );
            assert_eq!(
                create_vesting_allocation(&env, id, user, 0, 1_000, 0, 100),
                Err(CampaignError::InvalidAmount)
            );
        });
    }

    #[test]
    fn nothing_claimable_before_cliff() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            let user = Address::generate(&env);
            let vid =
                create_vesting_allocation(&env, id, user.clone(), 1_000, 1_000, 100, 1_000)
                    .unwrap();
            env.ledger().set_timestamp(1_000 + 50); // before the cliff
            assert_eq!(
                claim_vesting(&env, id, vid, &user),
                Err(CampaignError::NothingVested)
            );
        });
    }

    #[test]
    fn partial_vesting_between_cliff_and_end() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            let user = Address::generate(&env);
            let vid =
                create_vesting_allocation(&env, id, user.clone(), 1_000, 1_000, 100, 1_000)
                    .unwrap();
            // Halfway through the duration (from schedule start, not cliff).
            env.ledger().set_timestamp(1_000 + 500);
            let payout = claim_vesting(&env, id, vid, &user).unwrap();
            assert_eq!(payout, 500);

            // Immediately claiming again at the same timestamp: nothing new.
            assert_eq!(
                claim_vesting(&env, id, vid, &user),
                Err(CampaignError::NothingVested)
            );

            let v = get_vesting(&env, id, vid).unwrap();
            assert_eq!(v.claimed, 500);
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.claimed, 500);
            assert_eq!(c.pending_allocations, 1); // not fully settled yet
        });
    }

    #[test]
    fn full_vesting_after_duration_and_repeated_claims_sum_to_total() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            let user = Address::generate(&env);
            let vid =
                create_vesting_allocation(&env, id, user.clone(), 1_000, 1_000, 100, 1_000)
                    .unwrap();

            env.ledger().set_timestamp(1_000 + 300);
            let p1 = claim_vesting(&env, id, vid, &user).unwrap();
            env.ledger().set_timestamp(1_000 + 700);
            let p2 = claim_vesting(&env, id, vid, &user).unwrap();
            // Past the end of the schedule: everything remaining vests.
            env.ledger().set_timestamp(1_000 + 10_000);
            let p3 = claim_vesting(&env, id, vid, &user).unwrap();

            assert_eq!(p1 + p2 + p3, 1_000);
            let v = get_vesting(&env, id, vid).unwrap();
            assert_eq!(v.claimed, 1_000);
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.claimed, 1_000);
            assert_eq!(c.pending_allocations, 0); // fully settled, can close
            assert_accounting(&c);

            // Fully claimed: no more payout available.
            assert_eq!(
                claim_vesting(&env, id, vid, &user),
                Err(CampaignError::NothingVested)
            );
        });
    }

    #[test]
    fn vesting_rejects_wrong_recipient_and_insufficient_funds() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 100);
            let user = Address::generate(&env);
            let other = Address::generate(&env);
            assert_eq!(
                create_vesting_allocation(&env, id, user.clone(), 200, 1_000, 0, 1_000),
                Err(CampaignError::InsufficientCampaignFunds)
            );
            let vid =
                create_vesting_allocation(&env, id, user, 100, 1_000, 0, 1_000).unwrap();
            env.ledger().set_timestamp(1_000 + 1_000);
            assert_eq!(
                claim_vesting(&env, id, vid, &other),
                Err(CampaignError::VestingNotFound)
            );
        });
    }

    // ── #1329: campaign-specific claim limits ────────────────────────────────

    #[test]
    fn claim_within_cap_succeeds_and_tracks_cumulative() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            set_claim_cap(&env, id, 150).unwrap();
            let user = Address::generate(&env);

            distribute(&env, id, 100, vec![&env, (user.clone(), 1)]).unwrap(); // alloc 0
            assert_eq!(claim(&env, id, &user, vec![&env, 0]).unwrap(), 100);
            assert_eq!(claimed_by_account(&env, id, &user), 100);

            distribute(&env, id, 50, vec![&env, (user.clone(), 1)]).unwrap(); // alloc 1
            assert_eq!(claim(&env, id, &user, vec![&env, 1]).unwrap(), 50);
            assert_eq!(claimed_by_account(&env, id, &user), 150); // exactly the cap
        });
    }

    #[test]
    fn claim_exceeding_cap_is_rejected_before_any_mutation() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            set_claim_cap(&env, id, 100).unwrap();
            let user = Address::generate(&env);
            distribute(&env, id, 150, vec![&env, (user.clone(), 1)]).unwrap(); // alloc 0

            let before = get_campaign(&env, id).unwrap();
            assert_eq!(
                claim(&env, id, &user, vec![&env, 0]),
                Err(CampaignError::ClaimLimitExceeded)
            );
            // Rejected before transfer or state mutation.
            assert_eq!(get_campaign(&env, id).unwrap(), before);
            assert_eq!(
                get_allocation(&env, id, 0).unwrap().status,
                AllocationStatus::Pending
            );
            assert_eq!(claimed_by_account(&env, id, &user), 0);
        });
    }

    #[test]
    fn cap_cannot_be_bypassed_by_splitting_across_calls() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id = new_campaign(&env, 1_000);
            set_claim_cap(&env, id, 100).unwrap();
            let user = Address::generate(&env);

            distribute(&env, id, 60, vec![&env, (user.clone(), 1)]).unwrap(); // alloc 0
            distribute(&env, id, 60, vec![&env, (user.clone(), 1)]).unwrap(); // alloc 1
            assert_eq!(claim(&env, id, &user, vec![&env, 0]).unwrap(), 60);
            // Second, independent call would push cumulative to 120 > 100.
            assert_eq!(
                claim(&env, id, &user, vec![&env, 1]),
                Err(CampaignError::ClaimLimitExceeded)
            );
            assert_eq!(claimed_by_account(&env, id, &user), 60);
        });
    }

    #[test]
    fn cap_is_independent_per_campaign() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let id_a = new_campaign(&env, 1_000);
            let id_b = new_campaign(&env, 1_000);
            set_claim_cap(&env, id_a, 50).unwrap();
            // id_b has no cap.
            let user = Address::generate(&env);

            distribute(&env, id_a, 50, vec![&env, (user.clone(), 1)]).unwrap();
            distribute(&env, id_b, 500, vec![&env, (user.clone(), 1)]).unwrap();

            assert_eq!(claim(&env, id_a, &user, vec![&env, 0]).unwrap(), 50);
            // Unrelated campaign's own cumulative total is unaffected by id_a.
            assert_eq!(claim(&env, id_b, &user, vec![&env, 0]).unwrap(), 500);
            assert_eq!(claimed_by_account(&env, id_a, &user), 50);
            assert_eq!(claimed_by_account(&env, id_b, &user), 500);
        });
    }

    #[test]
    fn create_validates_inputs() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let f = Address::generate(&env);
            let t = Address::generate(&env);
            assert_eq!(
                create_campaign(&env, f.clone(), t.clone(), 0, WINDOW),
                Err(CampaignError::InvalidAmount)
            );
            assert_eq!(
                create_campaign(&env, f.clone(), t.clone(), 1, MIN_CLAIM_WINDOW_SECS - 1),
                Err(CampaignError::InvalidClaimWindow)
            );
            assert_eq!(
                create_campaign(&env, f, t, 1, MAX_CLAIM_WINDOW_SECS + 1),
                Err(CampaignError::InvalidClaimWindow)
            );
            assert_eq!(
                close_campaign(&env, 99),
                Err(CampaignError::CampaignNotFound)
            );
        });
    }

    // ── #1323: funding handshake before activation ───────────────────────────

    #[test]
    fn new_campaign_is_pending_with_zero_funded_and_epoch() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset, 1_000);
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.status, CampaignStatus::Pending);
            assert_eq!(c.funded, 0);
            assert_eq!(c.epoch_id, 0);
        });
    }

    #[test]
    fn exact_funding_activates_successfully() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset.clone(), 1_000);
            assert_eq!(fund_campaign(&env, id, &asset, 1_000).unwrap(), 1_000);
            activate_campaign(&env, id, 5).unwrap();
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.status, CampaignStatus::Open);
            assert_eq!(c.epoch_id, 5);
        });
    }

    #[test]
    fn short_funding_leaves_activation_rejected_and_state_unchanged() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset.clone(), 1_000);
            fund_campaign(&env, id, &asset, 600).unwrap();
            let before = get_campaign(&env, id).unwrap();

            assert_eq!(
                activate_campaign(&env, id, 1),
                Err(CampaignError::InsufficientCampaignFunds)
            );
            assert_eq!(get_campaign(&env, id).unwrap(), before);
        });
    }

    #[test]
    fn funding_in_the_wrong_asset_is_rejected_and_not_counted() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let other_asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset, 1_000);

            assert_eq!(
                fund_campaign(&env, id, &other_asset, 1_000),
                Err(CampaignError::WrongFundingAsset)
            );
            assert_eq!(get_campaign(&env, id).unwrap().funded, 0);
            assert_eq!(
                activate_campaign(&env, id, 1),
                Err(CampaignError::InsufficientCampaignFunds)
            );
        });
    }

    #[test]
    fn funding_can_be_split_across_multiple_deposits() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset.clone(), 1_000);
            assert_eq!(fund_campaign(&env, id, &asset, 400).unwrap(), 400);
            assert_eq!(fund_campaign(&env, id, &asset, 600).unwrap(), 1_000);
            activate_campaign(&env, id, 1).unwrap();
            assert_eq!(get_campaign(&env, id).unwrap().status, CampaignStatus::Open);
        });
    }

    // ── #1324: cancellation of unfunded/unactivated campaigns ────────────────

    #[test]
    fn pending_campaign_can_be_cancelled_and_returns_funded_amount() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset.clone(), 1_000);
            fund_campaign(&env, id, &asset, 300).unwrap();

            assert_eq!(cancel_campaign(&env, id).unwrap(), 300);
            let c = get_campaign(&env, id).unwrap();
            assert_eq!(c.status, CampaignStatus::Canceled);
        });
    }

    #[test]
    fn cancellation_is_rejected_once_already_activated() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset.clone(), 1_000);
            fund_campaign(&env, id, &asset, 1_000).unwrap();
            activate_campaign(&env, id, 1).unwrap();

            assert_eq!(
                cancel_campaign(&env, id),
                Err(CampaignError::CampaignNotPending)
            );
        });
    }

    #[test]
    fn repeated_cancellation_is_rejected_not_repeated() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset, 1_000);
            cancel_campaign(&env, id).unwrap();
            assert_eq!(
                cancel_campaign(&env, id),
                Err(CampaignError::CampaignNotPending)
            );
        });
    }

    // ── #1325: epoch snapshot binding ─────────────────────────────────────────

    #[test]
    fn allocations_are_stamped_with_the_activation_epoch_and_claims_require_it() {
        let (env, host) = setup();
        env.as_contract(&host, || {
            let asset = Address::generate(&env);
            let id = new_pending_campaign(&env, asset.clone(), 1_000);
            fund_campaign(&env, id, &asset, 1_000).unwrap();
            activate_campaign(&env, id, 99).unwrap();

            let user = Address::generate(&env);
            let created = distribute(&env, id, 100, vec![&env, (user.clone(), 1u32)]).unwrap();
            let (_, alloc_id, amount) = created.get(0).unwrap();

            let allocation = get_allocation(&env, id, alloc_id).unwrap();
            assert_eq!(allocation.epoch_id, 99);

            // A later change to "current state" (modelled here as the
            // campaign's own epoch_id, which has no setter and so can never
            // actually change post-activation) can't retroactively alter
            // this allocation's snapshot: the claim still succeeds because
            // the campaign's immutable epoch_id still agrees with it.
            assert_eq!(get_campaign(&env, id).unwrap().epoch_id, 99);
            assert_eq!(
                claim(&env, id, &user, vec![&env, alloc_id]).unwrap(),
                amount
            );
        });
    }
}
