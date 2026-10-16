//! Treasury guardrails for the fee collector.
//!
//! This module defines the guardrail configuration and the queued governance
//! timelock actions that gate sensitive treasury operations. It also specifies
//! the cancellation rules for queued timelock actions (see issue #1210) and the
//! revenue source allowlist that restricts fee deposits to registered sources
//! (see issue #1154).

use soroban_sdk::{contracttype, Address, Env, Vec};

/// Lifecycle states a queued governance timelock action can be in.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelockActionState {
    /// Queued and waiting for the timelock delay to elapse.
    Queued,
    /// Timelock delay elapsed; the action may be executed.
    Ready,
    /// The action has been executed. Terminal state.
    Executed,
    /// The action has been cancelled. Terminal state.
    Cancelled,
}

/// A queued governance timelock action.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelockAction {
    pub id: u64,
    /// Address that queued the action and is authorized to cancel it.
    pub proposer: Address,
    /// Address that governs the treasury (e.g. the governance contract).
    pub governor: Address,
    pub state: TimelockActionState,
    /// Ledger timestamp at which the action becomes executable.
    pub executable_at: u64,
}

/// Errors returned by the timelock cancellation and execution paths.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelockError {
    /// The action does not exist.
    NotFound,
    /// The caller is not authorized to cancel the action.
    Unauthorized,
    /// The action is not in a state from which cancellation is allowed.
    NotCancellable,
    /// The action is not in a state from which execution is allowed.
    NotExecutable,
}

/// Errors returned by the revenue source allowlist paths.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllowlistError {
    /// The caller is not authorized to mutate the allowlist.
    Unauthorized,
    /// The source is already registered in the allowlist.
    AlreadyRegistered,
    /// The source is not registered in the allowlist.
    NotRegistered,
    /// The source is not authorized to deposit fees.
    SourceNotAllowed,
}

/// Returns whether `state` is a lifecycle state from which cancellation is
/// permitted. Only `Queued` and `Ready` actions may be cancelled; `Executed`
/// and `Cancelled` are terminal states and can never be cancelled again.
pub fn is_cancellable(state: TimelockActionState) -> bool {
    matches!(state, TimelockActionState::Queued | TimelockActionState::Ready)
}

/// Returns whether `state` is a lifecycle state from which execution is
/// permitted. Cancelled actions can never be executed later.
pub fn is_executable(state: TimelockActionState) -> bool {
    matches!(state, TimelockActionState::Ready)
}

/// Cancels a queued governance timelock action.
///
/// Cancellation authority: only the action's `proposer` or the treasury
/// `governor` may cancel. Cancellation is allowed only while the action is in
/// the `Queued` or `Ready` state. Once cancelled, the action transitions to the
/// terminal `Cancelled` state and can never be executed later.
pub fn cancel_action(
    env: &Env,
    actions: &mut Vec<TimelockAction>,
    action_id: u64,
    caller: &Address,
) -> Result<(), TimelockError> {
    caller.require_auth();

    let mut index: Option<u32> = None;
    for i in 0..actions.len() {
        if actions.get(i).unwrap().id == action_id {
            index = Some(i);
            break;
        }
    }

    let i = index.ok_or(TimelockError::NotFound)?;
    let mut action = actions.get(i).unwrap();

    if caller != &action.proposer && caller != &action.governor {
        return Err(TimelockError::Unauthorized);
    }

    if !is_cancellable(action.state) {
        return Err(TimelockError::NotCancellable);
    }

    action.state = TimelockActionState::Cancelled;
    actions.set(i, action);
    env.storage().instance().set(&action_id, &TimelockActionState::Cancelled);

    Ok(())
}

/// Executes a queued governance timelock action.
///
/// Execution is allowed only from the `Ready` state. Cancelled actions are
/// rejected here, guaranteeing a cancelled action can never be executed later.
pub fn execute_action(
    env: &Env,
    actions: &mut Vec<TimelockAction>,
    action_id: u64,
) -> Result<(), TimelockError> {
    let mut index: Option<u32> = None;
    for i in 0..actions.len() {
        if actions.get(i).unwrap().id == action_id {
            index = Some(i);
            break;
        }
    }

    let i = index.ok_or(TimelockError::NotFound)?;
    let mut action = actions.get(i).unwrap();

    if !is_executable(action.state) {
        return Err(TimelockError::NotExecutable);
    }

    action.state = TimelockActionState::Executed;
    actions.set(i, action);
    env.storage().instance().set(&action_id, &TimelockActionState::Executed);

    Ok(())
}

/// Returns whether `source` is registered in the revenue source allowlist.
pub fn is_allowed_source(sources: &Vec<Address>, source: &Address) -> bool {
    for i in 0..sources.len() {
        if &sources.get(i).unwrap() == source {
            return true;
        }
    }
    false
}

/// Registers `source` in the revenue source allowlist.
///
/// Authorization: only the treasury `governor` may mutate the allowlist. The
/// change is evented so off-chain accounting and attribution reports can track
/// which sources were authorized and when. Registering an already-registered
/// source is rejected to keep the allowlist free of duplicates.
pub fn add_allowed_source(
    env: &Env,
    sources: &mut Vec<Address>,
    governor: &Address,
    caller: &Address,
    source: &Address,
) -> Result<(), AllowlistError> {
    caller.require_auth();

    if caller != governor {
        return Err(AllowlistError::Unauthorized);
    }

    if is_allowed_source(sources, source) {
        return Err(AllowlistError::AlreadyRegistered);
    }

    sources.push_back(source.clone());
    env.events()
        .publish((soroban_sdk::symbol_short!("src_add"),), source.clone());

    Ok(())
}

/// Removes `source` from the revenue source allowlist.
///
/// Authorization: only the treasury `governor` may mutate the allowlist. The
/// change is evented. Removal takes effect immediately: any subsequent deposit
/// from `source` fails before a token transfer is attempted, even if the source
/// was mid-way through an in-flight accounting period.
pub fn remove_allowed_source(
    env: &Env,
    sources: &mut Vec<Address>,
    governor: &Address,
    caller: &Address,
    source: &Address,
) -> Result<(), AllowlistError> {
    caller.require_auth();

    if caller != governor {
        return Err(AllowlistError::Unauthorized);
    }

    let mut index: Option<u32> = None;
    for i in 0..sources.len() {
        if &sources.get(i).unwrap() == source {
            index = Some(i);
            break;
        }
    }

    let i = index.ok_or(AllowlistError::NotRegistered)?;
    sources.remove(i);
    env.events()
        .publish((soroban_sdk::symbol_short!("src_rm"),), source.clone());

    Ok(())
}

/// Guards a fee deposit, ensuring `source` is an authorized revenue source.
///
/// This must be called before any token transfer is attempted so that deposits
/// from unknown sources fail early and cannot contaminate accounting or
/// attribution reports. Registered sources keep their existing deposit flow.
pub fn require_allowed_source(
    sources: &Vec<Address>,
    source: &Address,
) -> Result<(), AllowlistError> {
    if !is_allowed_source(sources, source) {
        return Err(AllowlistError::SourceNotAllowed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn setup(env: &Env, state: TimelockActionState) -> (Vec<TimelockAction>, Address, Address) {
        let proposer = Address::generate(env);
        let governor = Address::generate(env);
        let mut actions = Vec::new(env);
        actions.push_back(TimelockAction {
            id: 1,
            proposer: proposer.clone(),
            governor: governor.clone(),
            state,
            executable_at: 0,
        });
        (actions, proposer, governor)
    }

    #[test]
    fn proposer_can_cancel_queued() {
        let env = Env::default();
        env.mock_all_auths();
        let (mut actions, proposer, _) = setup(&env, TimelockActionState::Queued);
        assert!(cancel_action(&env, &mut actions, 1, &proposer).is_ok());
        assert_eq!(actions.get(0).unwrap().state, TimelockActionState::Cancelled);
    }

    #[test]
    fn governor_can_cancel_ready() {
        let env = Env::default();
        env.mock_all_auths();
        let (mut actions, _, governor) = setup(&env, TimelockActionState::Ready);
        assert!(cancel_action(&env, &mut actions, 1, &governor).is_ok());
        assert_eq!(actions.get(0).unwrap().state, TimelockActionState::Cancelled);
    }

    #[test]
    fn unauthorized_caller_cannot_cancel() {
        let env = Env::default();
        env.mock_all_auths();
        let (mut actions, _, _) = setup(&env, TimelockActionState::Queued);
        let stranger = Address::generate(&env);
        assert_eq!(
            cancel_action(&env, &mut actions, 1, &stranger),
            Err(TimelockError::Unauthorized)
        );
        assert_eq!(actions.get(0).unwrap().state, TimelockActionState::Queued);
    }

    #[test]
    fn cannot_cancel_executed() {
        let env = Env::default();
        env.mock_all_auths();
        let (mut actions, proposer, _) = setup(&env, TimelockActionState::Executed);
        assert_eq!(
            cancel_action(&env, &mut actions, 1, &proposer),
            Err(TimelockError::NotCancellable)
        );
    }

    #[test]
    fn cannot_cancel_cancelled() {
        let env = Env::default();
        env.mock_all_auths();
        let (mut actions, proposer, _) = setup(&env, TimelockActionState::Cancelled);
        assert_eq!(
            cancel_action(&env, &mut actions, 1, &proposer),
            Err(TimelockError::NotCancellable)
        );
    }

    #[test]
    fn cancelled_action_cannot_be_executed() {
        let env = Env::default();
        env.mock_all_auths();
        let (mut actions, proposer, _) = setup(&env, TimelockActionState::Ready);
        assert!(cancel_action(&env, &mut actions, 1, &proposer).is_ok());
        assert_eq!(
            execute_action(&env, &mut actions, 1),
            Err(TimelockError::NotExecutable)
        );
        assert_eq!(actions.get(0).unwrap().state, TimelockActionState::Cancelled);
    }

    #[test]
    fn ready_action_can_be_executed() {
        let env = Env::default();
        env.mock_all_auths();
        let (mut actions, _, _) = setup(&env, TimelockActionState::Ready);
        assert!(execute_action(&env, &mut actions, 1).is_ok());
        assert_eq!(actions.get(0).unwrap().state, TimelockActionState::Executed);
    }

    #[test]
    fn governor_can_add_and_remove_source() {
        let env = Env::default();
        env.mock_all_auths();
        let governor = Address::generate(&env);
        let source = Address::generate(&env);
        let mut sources = Vec::new(&env);

        assert!(add_allowed_source(&env, &mut sources, &governor, &governor, &source).is_ok());
        assert!(is_allowed_source(&sources, &source));

        assert!(remove_allowed_source(&env, &mut sources, &governor, &governor, &source).is_ok());
        assert!(!is_allowed_source(&sources, &source));
    }

    #[test]
    fn unauthorized_caller_cannot_mutate_allowlist() {
        let env = Env::default();
        env.mock_all_auths();
        let governor = Address::generate(&env);
        let stranger = Address::generate(&env);
        let source = Address::generate(&env);
        let mut sources = Vec::new(&env);

        assert_eq!(
            add_allowed_source(&env, &mut sources, &governor, &stranger, &source),
            Err(AllowlistError::Unauthorized)
        );
        assert_eq!(
            remove_allowed_source(&env, &mut sources, &governor, &stranger, &source),
            Err(AllowlistError::Unauthorized)
        );
    }

    #[test]
    fn duplicate_add_and_missing_remove_are_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let governor = Address::generate(&env);
        let source = Address::generate(&env);
        let mut sources = Vec::new(&env);

        assert!(add_allowed_source(&env, &mut sources, &governor, &governor, &source).is_ok());
        assert_eq!(
            add_allowed_source(&env, &mut sources, &governor, &governor, &source),
            Err(AllowlistError::AlreadyRegistered)
        );
        assert_eq!(
            remove_allowed_source(&env, &mut sources, &governor, &governor, &source),
            Ok(())
        );
        assert_eq!(
            remove_allowed_source(&env, &mut sources, &governor, &governor, &source),
            Err(AllowlistError::NotRegistered)
        );
    }

    #[test]
    fn unknown_source_deposit_fails_before_transfer() {
        let env = Env::default();
        env.mock_all_auths();
        let governor = Address::generate(&env);
        let known = Address::generate(&env);
        let unknown = Address::generate(&env);
        let mut sources = Vec::new(&env);

        assert!(add_allowed_source(&env, &mut sources, &governor, &governor, &known).is_ok());

        // Registered source remains compatible.
        assert!(require_allowed_source(&sources, &known).is_ok());
        // Unknown source is rejected before any transfer is attempted.
        assert_eq!(
            require_allowed_source(&sources, &unknown),
            Err(AllowlistError::SourceNotAllowed)
        );
    }

    #[test]
    fn removal_during_in_flight_period_blocks_further_deposits() {
        let env = Env::default();
        env.mock_all_auths();
        let governor = Address::generate(&env);
        let source = Address::generate(&env);
        let mut sources = Vec::new(&env);

        assert!(add_allowed_source(&env, &mut sources, &governor, &governor, &source).is_ok());
        assert!(require_allowed_source(&sources, &source).is_ok());

        // Removal takes effect immediately, even mid accounting period.
        assert!(remove_allowed_source(&env, &mut sources, &governor, &governor, &source).is_ok());
        assert_eq!(
            require_allowed_source(&sources, &source),
            Err(AllowlistError::SourceNotAllowed)
        );
    }
}
