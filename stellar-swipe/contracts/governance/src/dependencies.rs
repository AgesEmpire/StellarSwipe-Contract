//! Governance proposal dependency validation (#1128).
//!
//! Proposals may declare dependencies on other proposals. Dependencies are
//! validated at submission (existence, not expired, no cycles) and again at
//! execution (every dependency must be finalized).

use soroban_sdk::{contracterror, contracttype, Env, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DependencyError {
    DependencyMissing = 1,
    DependencyExpired = 2,
    DependencyNotFinalized = 3,
    DependencyCycle = 4,
    SelfDependency = 5,
    TooManyDependencies = 6,
    ProposalExists = 7,
    ProposalNotFound = 8,
}

pub const MAX_DEPENDENCIES: u32 = 10;
/// Bound on graph traversal to keep validation cost predictable.
const MAX_VISITS: u32 = 100;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepStatus {
    Pending,
    Finalized,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepMeta {
    pub id: u64,
    pub depends_on: Vec<u64>,
    pub status: DepStatus,
}

#[contracttype]
enum DepKey {
    Meta(u64),
}

pub fn get_meta(env: &Env, id: u64) -> Option<DepMeta> {
    env.storage().persistent().get(&DepKey::Meta(id))
}

fn save(env: &Env, m: &DepMeta) {
    env.storage().persistent().set(&DepKey::Meta(m.id), m);
}

/// Returns true if `target` is reachable from `start` via dependency edges.
fn reaches(env: &Env, start: u64, target: u64) -> Result<bool, DependencyError> {
    let mut stack: Vec<u64> = Vec::new(env);
    stack.push_back(start);
    let mut visits = 0u32;
    while let Some(cur) = stack.pop_back() {
        if cur == target {
            return Ok(true);
        }
        visits += 1;
        if visits > MAX_VISITS {
            return Err(DependencyError::TooManyDependencies);
        }
        if let Some(m) = get_meta(env, cur) {
            for d in m.depends_on.iter() {
                stack.push_back(d);
            }
        }
    }
    Ok(false)
}

pub fn submit(env: &Env, id: u64, depends_on: Vec<u64>) -> Result<(), DependencyError> {
    if get_meta(env, id).is_some() {
        return Err(DependencyError::ProposalExists);
    }
    if depends_on.len() > MAX_DEPENDENCIES {
        return Err(DependencyError::TooManyDependencies);
    }
    for d in depends_on.iter() {
        if d == id {
            return Err(DependencyError::SelfDependency);
        }
        let m = get_meta(env, d).ok_or(DependencyError::DependencyMissing)?;
        if m.status == DepStatus::Expired {
            return Err(DependencyError::DependencyExpired);
        }
        if reaches(env, d, id)? {
            return Err(DependencyError::DependencyCycle);
        }
    }
    save(env, &DepMeta { id, depends_on, status: DepStatus::Pending });
    Ok(())
}

/// Adds a dependency edge to an existing pending proposal, rejecting cycles.
pub fn add_dependency(env: &Env, id: u64, dep: u64) -> Result<(), DependencyError> {
    let mut m = get_meta(env, id).ok_or(DependencyError::ProposalNotFound)?;
    if dep == id {
        return Err(DependencyError::SelfDependency);
    }
    if m.depends_on.len() >= MAX_DEPENDENCIES {
        return Err(DependencyError::TooManyDependencies);
    }
    get_meta(env, dep).ok_or(DependencyError::DependencyMissing)?;
    if reaches(env, dep, id)? {
        return Err(DependencyError::DependencyCycle);
    }
    m.depends_on.push_back(dep);
    save(env, &m);
    Ok(())
}

pub fn set_status(env: &Env, id: u64, status: DepStatus) -> Result<(), DependencyError> {
    let mut m = get_meta(env, id).ok_or(DependencyError::ProposalNotFound)?;
    m.status = status;
    save(env, &m);
    Ok(())
}

/// Must pass before a proposal may enter an executable state.
pub fn validate_for_execution(env: &Env, id: u64) -> Result<(), DependencyError> {
    let m = get_meta(env, id).ok_or(DependencyError::ProposalNotFound)?;
    for d in m.depends_on.iter() {
        let dm = get_meta(env, d).ok_or(DependencyError::DependencyMissing)?;
        match dm.status {
            DepStatus::Finalized => {}
            DepStatus::Expired => return Err(DependencyError::DependencyExpired),
            DepStatus::Pending => return Err(DependencyError::DependencyNotFinalized),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, vec, Env};

    #[contract]
    struct Dummy;

    #[test]
    fn linear_missing_and_cyclic() {
        let env = Env::default();
        let cid = env.register(Dummy, ());
        env.as_contract(&cid, || {
            submit(&env, 1, vec![&env]).unwrap();
            submit(&env, 2, vec![&env, 1]).unwrap();
            submit(&env, 3, vec![&env, 2]).unwrap();
            assert_eq!(submit(&env, 9, vec![&env, 42]), Err(DependencyError::DependencyMissing));
            assert_eq!(add_dependency(&env, 1, 3), Err(DependencyError::DependencyCycle));
            assert_eq!(add_dependency(&env, 1, 1), Err(DependencyError::SelfDependency));
            assert_eq!(validate_for_execution(&env, 3), Err(DependencyError::DependencyNotFinalized));
            set_status(&env, 2, DepStatus::Finalized).unwrap();
            validate_for_execution(&env, 3).unwrap();
            set_status(&env, 1, DepStatus::Expired).unwrap();
            assert_eq!(validate_for_execution(&env, 2), Err(DependencyError::DependencyExpired));
            assert_eq!(submit(&env, 4, vec![&env, 1]), Err(DependencyError::DependencyExpired));
        });
    }
}
