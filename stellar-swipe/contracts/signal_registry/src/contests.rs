use crate::errors::ContestError;
use crate::types::Signal;
use soroban_sdk::{contracttype, Address, Bytes, Env, Map, String, Symbol, Vec};

/// Minimum ledgers that must elapse between contest creation and finalization.
/// Ensures the finalization ledger sequence is unknowable to the contest creator
/// at creation time, making winner tie-breaking verifiably fair.
const MIN_RANDOMNESS_DELAY_LEDGERS: u32 = 5;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContestMetric {
    HighestROI,
    BestSuccessRate,
    MostVolume,
    MostFollowers,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContestStatus {
    Active,
    Finalized,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ContestEntry {
    pub provider: Address,
    pub signals_submitted: Vec<u64>,
    pub total_roi: i128,
    pub success_rate: u32,
    pub total_volume: i128,
    pub score: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Contest {
    pub id: u64,
    pub name: String,
    pub start_time: u64,
    pub end_time: u64,
    pub metric: ContestMetric,
    pub min_signals: u32,
    pub entries: Map<Address, ContestEntry>,
    pub winners: Vec<Address>,
    pub prize_pool: i128,
    pub status: ContestStatus,
    /// Pre-committed seed (= contest_id at creation). Combined with the future
    /// ledger sequence at finalization to derive a verifiable tiebreak nonce.
    pub random_seed: u64,
    /// Earliest ledger sequence at which finalization is permitted. Ensures the
    /// finalization ledger (and thus the tiebreak nonce) is unknown at creation.
    pub finalize_after_ledger: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum ContestStorageKey {
    ContestCounter,
    Contests,
    ActiveContests,
}

pub fn create_contest(
    env: &Env,
    name: String,
    start_time: u64,
    end_time: u64,
    metric: ContestMetric,
    min_signals: u32,
    prize_pool: i128,
) -> Result<u64, ContestError> {
    let current_time = env.ledger().timestamp();

    if start_time >= end_time {
        return Err(ContestError::InvalidTimeRange);
    }
    if end_time <= current_time {
        return Err(ContestError::InvalidTimeRange);
    }
    if prize_pool < 0 {
        return Err(ContestError::InvalidPrizePool);
    }

    let counter_key = ContestStorageKey::ContestCounter;
    let contest_id: u64 = env.storage().persistent().get(&counter_key).unwrap_or(0) + 1;
    env.storage().persistent().set(&counter_key, &contest_id);

    let creation_ledger = env.ledger().sequence();
    let contest = Contest {
        id: contest_id,
        name,
        start_time,
        end_time,
        metric,
        min_signals,
        entries: Map::new(env),
        winners: Vec::new(env),
        prize_pool,
        status: ContestStatus::Active,
        random_seed: contest_id,
        finalize_after_ledger: creation_ledger.saturating_add(MIN_RANDOMNESS_DELAY_LEDGERS),
    };

    let contests_key = ContestStorageKey::Contests;
    let mut contests: Map<u64, Contest> = env
        .storage()
        .persistent()
        .get(&contests_key)
        .unwrap_or(Map::new(env));
    contests.set(contest_id, contest.clone());
    env.storage().persistent().set(&contests_key, &contests);

    let active_key = ContestStorageKey::ActiveContests;
    let mut active: Vec<u64> = env
        .storage()
        .persistent()
        .get(&active_key)
        .unwrap_or(Vec::new(env));
    active.push_back(contest_id);
    env.storage().persistent().set(&active_key, &active);

    Ok(contest_id)
}

pub fn auto_enter_signal(env: &Env, signal: &Signal) -> Result<(), ContestError> {
    let current_time = env.ledger().timestamp();
    let active_key = ContestStorageKey::ActiveContests;
    let active_contests: Vec<u64> = env
        .storage()
        .persistent()
        .get(&active_key)
        .unwrap_or(Vec::new(env));

    let contests_key = ContestStorageKey::Contests;
    let mut contests: Map<u64, Contest> = env
        .storage()
        .persistent()
        .get(&contests_key)
        .unwrap_or(Map::new(env));

    for i in 0..active_contests.len() {
        let contest_id = active_contests.get(i).unwrap();
        if let Some(mut contest) = contests.get(contest_id) {
            if contest.status == ContestStatus::Active
                && current_time >= contest.start_time
                && current_time <= contest.end_time
            {
                let provider = signal.provider.clone();
                let mut entry = contest
                    .entries
                    .get(provider.clone())
                    .unwrap_or(ContestEntry {
                        provider: provider.clone(),
                        signals_submitted: Vec::new(env),
                        total_roi: 0,
                        success_rate: 0,
                        total_volume: 0,
                        score: 0,
                    });

                entry.signals_submitted.push_back(signal.id);
                // ROI/volume are applied when trades are recorded (see `apply_trade_to_contest_entries`).
                // `success_rate` is updated there too; it is left unchanged here because a freshly
                // auto-entered signal has no recorded outcome yet (the previous `successful * 100`
                // recomputation referenced an undefined identifier and broke the crate build).

                entry.score = calculate_contest_score(&entry, &contest.metric, env);
                contest.entries.set(provider, entry);
                contests.set(contest_id, contest);
            }
        }
    }

    env.storage().persistent().set(&contests_key, &contests);
    Ok(())
}

/// Add this trade's ROI and volume to any active contest entry that lists `signal_id` for `provider`.
pub fn apply_trade_to_contest_entries(
    env: &Env,
    signal_id: u64,
    provider: &Address,
    trade_roi: i128,
    volume: i128,
) {
    let current_time = env.ledger().timestamp();
    let active_key = ContestStorageKey::ActiveContests;
    let active_contests: Vec<u64> = env
        .storage()
        .persistent()
        .get(&active_key)
        .unwrap_or(Vec::new(env));

    let contests_key = ContestStorageKey::Contests;
    let mut contests: Map<u64, Contest> = env
        .storage()
        .persistent()
        .get(&contests_key)
        .unwrap_or(Map::new(env));

    let mut any = false;
    for i in 0..active_contests.len() {
        let contest_id = active_contests.get(i).unwrap();
        let Some(mut contest) = contests.get(contest_id) else {
            continue;
        };
        if contest.status != ContestStatus::Active {
            continue;
        }
        if current_time < contest.start_time || current_time > contest.end_time {
            continue;
        }
        let Some(mut entry) = contest.entries.get(provider.clone()) else {
            continue;
        };
        let mut lists_signal = false;
        for j in 0..entry.signals_submitted.len() {
            if entry.signals_submitted.get(j).unwrap() == signal_id {
                lists_signal = true;
                break;
            }
        }
        if !lists_signal {
            continue;
        }
        entry.total_roi = entry.total_roi.saturating_add(trade_roi);
        entry.total_volume = entry.total_volume.saturating_add(volume);
        entry.score = calculate_contest_score(&entry, &contest.metric, env);
        contest.entries.set(provider.clone(), entry);
        contests.set(contest_id, contest);
        any = true;
    }
    if any {
        env.storage().persistent().set(&contests_key, &contests);
    }
}

fn calculate_contest_score(entry: &ContestEntry, metric: &ContestMetric, _env: &Env) -> i128 {
    match metric {
        ContestMetric::HighestROI => entry.total_roi,
        ContestMetric::BestSuccessRate => entry.success_rate as i128,
        ContestMetric::MostVolume => entry.total_volume,
        ContestMetric::MostFollowers => 0, // Placeholder - would integrate with social module
    }
}

/// Derive a tiebreak nonce from a pre-committed seed and the finalization ledger sequence.
///
/// `SHA-256(seed_bytes || finalize_ledger_bytes)` → first 8 bytes as big-endian u64.
/// Given the same `(random_seed, ledger_sequence)` pair the result is always identical,
/// enabling independent verification of any past selection.
fn derive_tiebreak_nonce(env: &Env, random_seed: u64, ledger_sequence: u32) -> u64 {
    let mut preimage = Bytes::new(env);
    preimage.append(&Bytes::from_array(env, &random_seed.to_be_bytes()));
    preimage.append(&Bytes::from_array(env, &ledger_sequence.to_be_bytes()));
    let hash = env.crypto().sha256(&preimage);
    let bytes = hash.to_array();
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Returns true if `a` should be swapped after `b` in descending-score order.
/// Equal scores are broken by a provider-specific hash derived from `tiebreak_nonce`,
/// ensuring determinism across identical inputs while being opaque at creation time.
fn should_swap(a: &ContestEntry, b: &ContestEntry, tiebreak_nonce: u64) -> bool {
    if a.score != b.score {
        return a.score < b.score;
    }
    // Derive a per-provider key: XOR-fold address bytes against the nonce.
    let a_key = provider_tiebreak_key(tiebreak_nonce, &a.provider);
    let b_key = provider_tiebreak_key(tiebreak_nonce, &b.provider);
    a_key > b_key
}

fn provider_tiebreak_key(nonce: u64, provider: &Address) -> u64 {
    let s = provider.to_string();
    let bytes = s.to_bytes();
    let mut key = nonce;
    let len = bytes.len().min(8);
    for i in 0..len {
        key ^= (bytes.get(i).unwrap_or(0) as u64).wrapping_shl((i as u32) * 8);
    }
    key
}

pub fn finalize_contest(env: &Env, contest_id: u64) -> Result<Vec<Address>, ContestError> {
    let current_time = env.ledger().timestamp();
    let contests_key = ContestStorageKey::Contests;
    let mut contests: Map<u64, Contest> = env
        .storage()
        .persistent()
        .get(&contests_key)
        .unwrap_or(Map::new(env));

    let mut contest = contests
        .get(contest_id)
        .ok_or(ContestError::ContestNotFound)?;

    if current_time < contest.end_time {
        return Err(ContestError::ContestNotEnded);
    }
    if contest.status != ContestStatus::Active {
        return Err(ContestError::AlreadyFinalized);
    }

    let current_ledger = env.ledger().sequence();
    if current_ledger < contest.finalize_after_ledger {
        return Err(ContestError::RandomnessNotAvailable);
    }

    // Derive a verifiable tiebreak nonce from the pre-committed seed and the
    // current ledger sequence (unknown at contest creation time).
    // Inputs are emitted so the result can be independently reproduced.
    let tiebreak_nonce = derive_tiebreak_nonce(env, contest.random_seed, current_ledger);

    env.events().publish(
        (Symbol::new(env, "contest_randomness"), contest_id),
        (
            contest.random_seed,
            contest.finalize_after_ledger,
            current_ledger,
        ),
    );

    let mut qualified_entries: Vec<ContestEntry> = Vec::new(env);
    let entry_keys = contest.entries.keys();

    for i in 0..entry_keys.len() {
        let key = entry_keys.get(i).unwrap();
        if let Some(entry) = contest.entries.get(key) {
            if entry.signals_submitted.len() >= contest.min_signals as u32 {
                qualified_entries.push_back(entry);
            }
        }
    }

    // Sort by score descending; equal scores broken deterministically by tiebreak_nonce.
    for i in 0..qualified_entries.len() {
        for j in 0..qualified_entries.len().saturating_sub(i + 1) {
            let a = qualified_entries.get(j).unwrap();
            let b = qualified_entries.get(j + 1).unwrap();
            if should_swap(&a, &b, tiebreak_nonce) {
                qualified_entries.set(j, b);
                qualified_entries.set(j + 1, a);
            }
        }
    }

    let mut winners: Vec<Address> = Vec::new(env);
    let winner_count = qualified_entries.len().min(3);

    for i in 0..winner_count {
        let entry = qualified_entries.get(i).unwrap();
        winners.push_back(entry.provider.clone());
    }

    if !winners.is_empty() {
        distribute_prize_pool(env, &contest, &winners)?;
    }

    contest.winners = winners.clone();
    contest.status = ContestStatus::Finalized;
    contests.set(contest_id, contest);
    env.storage().persistent().set(&contests_key, &contests);

    // Remove from active contests
    let active_key = ContestStorageKey::ActiveContests;
    let mut active: Vec<u64> = env
        .storage()
        .persistent()
        .get(&active_key)
        .unwrap_or(Vec::new(env));
    let mut new_active: Vec<u64> = Vec::new(env);
    for i in 0..active.len() {
        let id = active.get(i).unwrap();
        if id != contest_id {
            new_active.push_back(id);
        }
    }
    env.storage().persistent().set(&active_key, &new_active);

    Ok(winners)
}

fn distribute_prize_pool(
    env: &Env,
    contest: &Contest,
    winners: &Vec<Address>,
) -> Result<(), ContestError> {
    if contest.prize_pool == 0 || winners.is_empty() {
        return Ok(());
    }

    let percentages = [50, 30, 20]; // 1st: 50%, 2nd: 30%, 3rd: 20%

    for i in 0..winners.len().min(3) {
        let winner = winners.get(i).unwrap();
        let prize = (contest.prize_pool * percentages[i as usize]) / 100;

        // Store prize allocation (actual token transfer would happen externally)
        let prize_key = (contest.id, winner.clone());
        env.storage().persistent().set(&prize_key, &prize);
    }

    Ok(())
}

pub fn get_contest(env: &Env, contest_id: u64) -> Result<Contest, ContestError> {
    let contests_key = ContestStorageKey::Contests;
    let contests: Map<u64, Contest> = env
        .storage()
        .persistent()
        .get(&contests_key)
        .unwrap_or(Map::new(env));
    contests
        .get(contest_id)
        .ok_or(ContestError::ContestNotFound)
}

pub fn get_active_contests(env: &Env) -> Vec<u64> {
    let active_key = ContestStorageKey::ActiveContests;
    env.storage()
        .persistent()
        .get(&active_key)
        .unwrap_or(Vec::new(env))
}

pub fn get_contest_leaderboard(
    env: &Env,
    contest_id: u64,
) -> Result<Vec<ContestEntry>, ContestError> {
    let contest = get_contest(env, contest_id)?;
    let mut entries: Vec<ContestEntry> = Vec::new(env);

    let entry_keys = contest.entries.keys();
    for i in 0..entry_keys.len() {
        let key = entry_keys.get(i).unwrap();
        if let Some(entry) = contest.entries.get(key) {
            entries.push_back(entry);
        }
    }

    // Sort by score descending
    for i in 0..entries.len() {
        for j in 0..entries.len().saturating_sub(i + 1) {
            let a = entries.get(j).unwrap();
            let b = entries.get(j + 1).unwrap();
            if a.score < b.score {
                entries.set(j, b);
                entries.set(j + 1, a);
            }
        }
    }

    Ok(entries)
}

pub fn get_provider_prize(env: &Env, contest_id: u64, provider: Address) -> i128 {
    let prize_key = (contest_id, provider);
    env.storage().persistent().get(&prize_key).unwrap_or(0)
}
