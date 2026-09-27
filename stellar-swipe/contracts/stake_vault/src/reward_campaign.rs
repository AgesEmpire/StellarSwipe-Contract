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
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CampaignStatus {
    Open = 0,
    Closed = 1,
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

/// Record a new open campaign.  The caller must transfer `budget` of `asset`
/// from `funder` into the contract in the same invocation.
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
        status: CampaignStatus::Open,
        created_at: env.ledger().timestamp(),
        closed_at: 0,
    };
    save(env, &campaign);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_CAMPAIGN_CREATED()),
        (campaign_id, funder, asset, budget, claim_window_secs),
    );
    Ok(campaign_id)
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
        total = add(total, allocation.amount)?;
        allocations.push_back(allocation);
    }

    for i in 0..allocations.len() {
        let mut allocation = allocations.get(i).unwrap();
        allocation.status = AllocationStatus::Claimed;
        env.storage().persistent().set(
            &CampaignKey::Allocation(campaign_id, allocation.allocation_id),
            &allocation,
        );
    }
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

    fn new_campaign(env: &Env, budget: i128) -> u64 {
        create_campaign(
            env,
            Address::generate(env),
            Address::generate(env),
            budget,
            WINDOW,
        )
        .unwrap()
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
}
