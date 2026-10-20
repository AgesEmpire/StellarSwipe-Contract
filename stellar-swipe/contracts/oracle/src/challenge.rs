//! Oracle observation challenge window (#1124).
//!
//! Observations are opened with a bounded challenge window. Whitelisted
//! challengers may submit evidence while the window is open. The admin
//! resolves challenges deterministically; unchallenged observations expire
//! into a final, usable state once the window elapses.

use soroban_sdk::{contracterror, contracttype, symbol_short, Address, BytesN, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ChallengeError {
    ObservationExists = 1,
    ObservationNotFound = 2,
    InvalidWindow = 3,
    NotChallenger = 4,
    WindowClosed = 5,
    WindowOpen = 6,
    InvalidState = 7,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationState {
    Open,
    Challenged,
    Resolved,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    pub id: u64,
    pub price: i128,
    pub opened_at: u64,
    pub window_end: u64,
    pub state: ObservationState,
    pub challenger: Option<Address>,
    pub evidence: Option<BytesN<32>>,
    /// Set on resolution: true if the challenge was upheld (observation rejected).
    pub upheld: bool,
}

#[contracttype]
enum ChallengeKey {
    Observation(u64),
    Challenger(Address),
}

pub fn set_challenger(env: &Env, who: &Address, allowed: bool) {
    let key = ChallengeKey::Challenger(who.clone());
    if allowed {
        env.storage().persistent().set(&key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
}

pub fn is_challenger(env: &Env, who: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&ChallengeKey::Challenger(who.clone()))
        .unwrap_or(false)
}

fn load(env: &Env, id: u64) -> Result<Observation, ChallengeError> {
    env.storage()
        .persistent()
        .get(&ChallengeKey::Observation(id))
        .ok_or(ChallengeError::ObservationNotFound)
}

fn save(env: &Env, obs: &Observation) {
    env.storage()
        .persistent()
        .set(&ChallengeKey::Observation(obs.id), obs);
}

pub fn open_observation(
    env: &Env,
    id: u64,
    price: i128,
    window_secs: u64,
) -> Result<Observation, ChallengeError> {
    if window_secs == 0 {
        return Err(ChallengeError::InvalidWindow);
    }
    if env.storage().persistent().has(&ChallengeKey::Observation(id)) {
        return Err(ChallengeError::ObservationExists);
    }
    let now = env.ledger().timestamp();
    let obs = Observation {
        id,
        price,
        opened_at: now,
        window_end: now.saturating_add(window_secs),
        state: ObservationState::Open,
        challenger: None,
        evidence: None,
        upheld: false,
    };
    save(env, &obs);
    env.events().publish((symbol_short!("obs_open"), id), obs.window_end);
    Ok(obs)
}

pub fn challenge(
    env: &Env,
    challenger: &Address,
    id: u64,
    evidence: BytesN<32>,
) -> Result<(), ChallengeError> {
    challenger.require_auth();
    if !is_challenger(env, challenger) {
        return Err(ChallengeError::NotChallenger);
    }
    let mut obs = load(env, id)?;
    if obs.state != ObservationState::Open {
        return Err(ChallengeError::InvalidState);
    }
    if env.ledger().timestamp() >= obs.window_end {
        return Err(ChallengeError::WindowClosed);
    }
    obs.state = ObservationState::Challenged;
    obs.challenger = Some(challenger.clone());
    obs.evidence = Some(evidence);
    save(env, &obs);
    env.events()
        .publish((symbol_short!("obs_chal"), id), challenger.clone());
    Ok(())
}

/// Caller must have verified admin auth. Deterministic: the outcome is the
/// supplied verdict, and a challenged observation can be resolved exactly once.
pub fn resolve(env: &Env, id: u64, upheld: bool) -> Result<Observation, ChallengeError> {
    let mut obs = load(env, id)?;
    if obs.state != ObservationState::Challenged {
        return Err(ChallengeError::InvalidState);
    }
    obs.state = ObservationState::Resolved;
    obs.upheld = upheld;
    save(env, &obs);
    env.events().publish((symbol_short!("obs_res"), id), upheld);
    Ok(obs)
}

/// Permissionless: finalizes an unchallenged observation after its window.
pub fn expire(env: &Env, id: u64) -> Result<Observation, ChallengeError> {
    let mut obs = load(env, id)?;
    if obs.state != ObservationState::Open {
        return Err(ChallengeError::InvalidState);
    }
    if env.ledger().timestamp() < obs.window_end {
        return Err(ChallengeError::WindowOpen);
    }
    obs.state = ObservationState::Expired;
    save(env, &obs);
    env.events().publish((symbol_short!("obs_exp"), id), obs.price);
    Ok(obs)
}

/// Settlement-sensitive paths may use an observation only once it is final
/// and not rejected by an upheld challenge.
pub fn usable_price(env: &Env, id: u64) -> Result<i128, ChallengeError> {
    let obs = load(env, id)?;
    match obs.state {
        ObservationState::Expired => Ok(obs.price),
        ObservationState::Resolved if !obs.upheld => Ok(obs.price),
        _ => Err(ChallengeError::InvalidState),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, Env};

    #[contract]
    struct Dummy;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(Dummy, ());
        (env, id)
    }

    #[test]
    fn expired_observation_is_usable() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            open_observation(&env, 1, 100, 60).unwrap();
            assert_eq!(expire(&env, 1), Err(ChallengeError::WindowOpen));
            env.ledger().with_mut(|l| l.timestamp += 60);
            expire(&env, 1).unwrap();
            assert_eq!(usable_price(&env, 1), Ok(100));
            let c = Address::generate(&env);
            set_challenger(&env, &c, true);
            let ev = BytesN::from_array(&env, &[0; 32]);
            assert_eq!(challenge(&env, &c, 1, ev), Err(ChallengeError::InvalidState));
        });
    }

    #[test]
    fn successful_challenge_rejects_observation() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            let c = Address::generate(&env);
            let ev = BytesN::from_array(&env, &[1; 32]);
            open_observation(&env, 2, 100, 60).unwrap();
            assert_eq!(
                challenge(&env, &c, 2, ev.clone()),
                Err(ChallengeError::NotChallenger)
            );
            set_challenger(&env, &c, true);
            challenge(&env, &c, 2, ev).unwrap();
            assert_eq!(usable_price(&env, 2), Err(ChallengeError::InvalidState));
            resolve(&env, 2, true).unwrap();
            assert_eq!(resolve(&env, 2, true), Err(ChallengeError::InvalidState));
            assert_eq!(usable_price(&env, 2), Err(ChallengeError::InvalidState));
        });
    }
}
