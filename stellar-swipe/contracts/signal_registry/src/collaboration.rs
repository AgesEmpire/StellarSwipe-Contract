use crate::errors::AdminError;
use crate::types::{Signal, SignalAction};
use soroban_sdk::{contracttype, Address, Env, Map, Vec};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollaborationStatus {
    PendingApproval,
    Approved,
    Rejected,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Author {
    pub address: Address,
    pub contribution_pct: u32, // Basis points (10000 = 100%)
    pub has_approved: bool,
}

#[contracttype]
#[derive(Clone)]
pub enum CollabStorageKey {
    CollaborativeSignals,
}

pub fn create_collaborative_signal(
    env: &Env,
    signal_id: u64,
    primary_author: Address,
    co_authors: Vec<Address>,
    contribution_pcts: Vec<u32>,
) -> Result<Vec<Author>, AdminError> {
    // Validate contributions sum to 100%
    let total: u32 = contribution_pcts.iter().sum();
    if total != 10000 {
        return Err(AdminError::InvalidParameter);
    }

    if co_authors.len() + 1 != contribution_pcts.len() {
        return Err(AdminError::InvalidParameter);
    }

    let mut authors = Vec::new(env);

    // Primary author auto-approves
    authors.push_back(Author {
        address: primary_author,
        contribution_pct: contribution_pcts.get(0).unwrap(),
        has_approved: true,
    });

    // Add co-authors (pending approval)
    for i in 0..co_authors.len() {
        authors.push_back(Author {
            address: co_authors.get(i).unwrap(),
            contribution_pct: contribution_pcts.get(i + 1).unwrap(),
            has_approved: false,
        });
    }

    store_collaborative_signal(env, signal_id, &authors);
    Ok(authors)
}

pub fn approve_collaborative_signal(
    env: &Env,
    signal_id: u64,
    approver: &Address,
) -> Result<bool, AdminError> {
    let mut authors =
        get_collaborative_signal(env, signal_id).ok_or(AdminError::InvalidParameter)?;

    let mut found = false;
    for i in 0..authors.len() {
        let mut author = authors.get(i).unwrap();
        if author.address == *approver {
            if author.has_approved {
                return Err(AdminError::InvalidParameter);
            }
            author.has_approved = true;
            authors.set(i, author);
            found = true;
            break;
        }
    }

    if !found {
        return Err(AdminError::Unauthorized);
    }

    store_collaborative_signal(env, signal_id, &authors);

    // Check if all approved
    let all_approved = (0..authors.len()).all(|i| authors.get(i).unwrap().has_approved);
    Ok(all_approved)
}

pub fn get_collaboration_status(authors: &Vec<Author>) -> CollaborationStatus {
    let all_approved = (0..authors.len()).all(|i| authors.get(i).unwrap().has_approved);
    if all_approved {
        CollaborationStatus::Approved
    } else {
        CollaborationStatus::PendingApproval
    }
}

pub fn distribute_collaborative_rewards(
    env: &Env,
    authors: &Vec<Author>,
    total_fees: i128,
    total_roi: i128,
) -> Vec<(Address, i128, i128)> {
    let mut distributions = Vec::new(env);
    let mut fee_sum = 0i128;
    let mut roi_sum = 0i128;

    // First pass: compute floor shares
    for i in 0..authors.len() {
        let author = authors.get(i).unwrap();
        let fee_share = (total_fees * author.contribution_pct as i128) / 10000;
        let roi_share = (total_roi * author.contribution_pct as i128) / 10000;
        fee_sum += fee_share;
        roi_sum += roi_share;
        distributions.push_back((author.address.clone(), fee_share, roi_share));
    }

    // Adjust the last entry to absorb any rounding remainder
    // (soroban_sdk::Vec has no `last_mut`; read, mutate, and write back by index).
    if distributions.len() > 0 {
        let last_idx = distributions.len() - 1;
        let (address, fee_share, roi_share) = distributions.get(last_idx).unwrap();
        let fee_rem = total_fees - fee_sum;
        let roi_rem = total_roi - roi_sum;
        distributions.set(
            last_idx,
            (address, fee_share + fee_rem, roi_share + roi_rem),
        );
    }

    distributions
}

fn store_collaborative_signal(env: &Env, signal_id: u64, authors: &Vec<Author>) {
    let mut map: Map<u64, Vec<Author>> = env
        .storage()
        .instance()
        .get(&CollabStorageKey::CollaborativeSignals)
        .unwrap_or(Map::new(env));
    map.set(signal_id, authors.clone());
    env.storage()
        .instance()
        .set(&CollabStorageKey::CollaborativeSignals, &map);
}

pub fn get_collaborative_signal(env: &Env, signal_id: u64) -> Option<Vec<Author>> {
    let map: Map<u64, Vec<Author>> = env
        .storage()
        .instance()
        .get(&CollabStorageKey::CollaborativeSignals)?;
    map.get(signal_id)
}

pub fn is_collaborative_signal(env: &Env, signal_id: u64) -> bool {
    get_collaborative_signal(env, signal_id).is_some()
}
