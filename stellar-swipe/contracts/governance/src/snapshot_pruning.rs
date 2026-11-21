//! Vote power snapshot pruning (#1129).
//!
//! Per-voter snapshots are stored per proposal. Once a proposal's challenge
//! and execution windows have elapsed, the admin may prune its snapshots in
//! bounded batches. An audit summary (voter count and total power) is retained
//! permanently. Pruning is idempotent and never touches active proposals.

use soroban_sdk::{contracterror, contracttype, Address, Env, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PruneError {
    Unauthorized = 1,
    ProposalNotFound = 2,
    ProposalActive = 3,
    InvalidLimit = 4,
}

pub const MAX_PRUNE_PER_CALL: u32 = 50;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotSet {
    pub proposal_id: u64,
    /// Timestamp after which challenge + execution windows have elapsed.
    pub retain_until: u64,
    pub voters: Vec<Address>,
}

/// Retained for audits after pruning.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotAudit {
    pub proposal_id: u64,
    pub voter_count: u32,
    pub total_power: i128,
    pub pruned: u32,
}

#[contracttype]
enum SnapKey {
    Set(u64),
    Power(u64, Address),
    Audit(u64),
}

pub fn record_snapshot(env: &Env, proposal_id: u64, retain_until: u64, voter: Address, power: i128) {
    let s = env.storage().persistent();
    let mut set: SnapshotSet = s.get(&SnapKey::Set(proposal_id)).unwrap_or(SnapshotSet {
        proposal_id,
        retain_until,
        voters: Vec::new(env),
    });
    let mut audit: SnapshotAudit = s.get(&SnapKey::Audit(proposal_id)).unwrap_or(SnapshotAudit {
        proposal_id,
        voter_count: 0,
        total_power: 0,
        pruned: 0,
    });
    let key = SnapKey::Power(proposal_id, voter.clone());
    let prev: i128 = s.get(&key).unwrap_or(0);
    if !s.has(&key) {
        set.voters.push_back(voter);
        audit.voter_count += 1;
    }
    audit.total_power = audit.total_power - prev + power;
    s.set(&key, &power);
    s.set(&SnapKey::Set(proposal_id), &set);
    s.set(&SnapKey::Audit(proposal_id), &audit);
}

pub fn get_power(env: &Env, proposal_id: u64, voter: &Address) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&SnapKey::Power(proposal_id, voter.clone()))
}

pub fn get_audit(env: &Env, proposal_id: u64) -> Option<SnapshotAudit> {
    env.storage().persistent().get(&SnapKey::Audit(proposal_id))
}

pub fn remaining(env: &Env, proposal_id: u64) -> u32 {
    env.storage()
        .persistent()
        .get::<_, SnapshotSet>(&SnapKey::Set(proposal_id))
        .map(|s| s.voters.len())
        .unwrap_or(0)
}

/// Prunes up to `limit` snapshots. Returns the number removed (0 once done).
pub fn prune(
    env: &Env,
    caller: &Address,
    admin: &Address,
    proposal_id: u64,
    limit: u32,
) -> Result<u32, PruneError> {
    caller.require_auth();
    if caller != admin {
        return Err(PruneError::Unauthorized);
    }
    if limit == 0 || limit > MAX_PRUNE_PER_CALL {
        return Err(PruneError::InvalidLimit);
    }
    let s = env.storage().persistent();
    let mut audit: SnapshotAudit = s
        .get(&SnapKey::Audit(proposal_id))
        .ok_or(PruneError::ProposalNotFound)?;
    let Some(mut set) = s.get::<_, SnapshotSet>(&SnapKey::Set(proposal_id)) else {
        return Ok(0); // already fully pruned
    };
    if env.ledger().timestamp() < set.retain_until {
        return Err(PruneError::ProposalActive);
    }
    let mut removed = 0u32;
    while removed < limit {
        let Some(v) = set.voters.pop_back() else { break };
        s.remove(&SnapKey::Power(proposal_id, v));
        removed += 1;
    }
    if set.voters.is_empty() {
        s.remove(&SnapKey::Set(proposal_id));
    } else {
        s.set(&SnapKey::Set(proposal_id), &set);
    }
    audit.pruned += removed;
    s.set(&SnapKey::Audit(proposal_id), &audit);
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, Env};

    #[contract]
    struct Dummy;

    #[test]
    fn bounded_safe_pruning() {
        let env = Env::default();
        env.mock_all_auths();
        let cid = env.register(Dummy, ());
        env.as_contract(&cid, || {
            let admin = Address::generate(&env);
            for _ in 0..7 {
                record_snapshot(&env, 1, 100, Address::generate(&env), 5);
            }
            assert_eq!(prune(&env, &admin, &admin, 1, 3), Err(PruneError::ProposalActive));
            env.ledger().with_mut(|l| l.timestamp = 100);
            let other = Address::generate(&env);
            assert_eq!(prune(&env, &other, &admin, 1, 3), Err(PruneError::Unauthorized));
            assert_eq!(prune(&env, &admin, &admin, 1, 3), Ok(3));
            assert_eq!(remaining(&env, 1), 4);
            assert_eq!(prune(&env, &admin, &admin, 1, 3), Ok(3));
            assert_eq!(prune(&env, &admin, &admin, 1, 3), Ok(1));
            assert_eq!(prune(&env, &admin, &admin, 1, 3), Ok(0));
            let a = get_audit(&env, 1).unwrap();
            assert_eq!((a.voter_count, a.total_power, a.pruned), (7, 35, 7));
        });
    }
}
