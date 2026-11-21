//! Treasury guardrails for the fee collector.
//!
//! This module defines the guardrail configuration and the queued governance
//! timelock actions that gate sensitive treasury operations. It also specifies
//! the cancellation rules for queued timelock actions (see issue #1210), the
//! revenue source allowlist that restricts fee deposits to registered sources
//! (see issue #1154), and the payout recipient rotation delay that prevents an
//! accidental or compromised admin update from redirecting accumulated fees
//! immediately (see issue #1157).

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

/// A pending payout recipient change awaiting its rotation delay.
///
/// The `recipient` is not used for payouts until the ledger reaches
/// `activates_at`. Until then the currently active recipient keeps receiving
/// fees, so a compromised or accidental admin update cannot redirect the fees
/// that accumulate during the delay window.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRecipient {
    /// The proposed payout recipient.
    pub recipient: Address,
    /// Address that proposed the change and is authorized to cancel it.
    pub proposer: Address,
    /// Ledger sequence at which the pending recipient becomes active.
    pub activates_at: u32,
}

/// Errors returned by the payout recipient rotation paths.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipientError {
    /// The caller is not authorized to mutate the payout recipient.
    Unauthorized,
    /// There is no pending recipient change to act on.
    NoPendingRecipient,
    /// The rotation delay has not yet elapsed.
    DelayNotElapsed,
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

/// Proposes a new payout recipient, storing it as pending until the rotation
/// delay elapses.
///
/// Authorization: only the treasury `governor` may propose a recipient change.
/// The proposal is evented and stored as a `PendingRecipient` with an
/// `activates_at` ledger of `current_ledger + delay`. The active recipient is
/// left untouched, so fees keep flowing to it during the delay window. A new
/// proposal replaces any previously pending recipient, restarting the delay.
pub fn propose_recipient(
    env: &Env,
    governor: &Address,
    caller: &Address,
    recipient: &Address,
    delay: u32,
) -> Result<PendingRecipient, RecipientError> {
    caller.require_auth();

    if caller != governor {
        return Err(RecipientError::Unauthorized);
    }

    let pending = PendingRecipient {
        recipient: recipient.clone(),
        proposer: caller.clone(),
        activates_at: env.ledger().sequence() + delay,
    };

    env.storage().instance().set(&PENDING_RECIPIENT, &pending);
    env.events().publish(
        (soroban_sdk::symbol_short!("rcpt_prop"),),
        (recipient.clone(), pending.activates_at),
    );

    Ok(pending)
}

/// Cancels a pending payout recipient change.
///
/// Authorization: only the pending change's `proposer` or the treasury
/// `governor` may cancel. Cancellation is evented and clears the pending
/// recipient, leaving the active recipient in place.
pub fn cancel_pending_recipient(
    env: &Env,
    governor: &Address,
    caller: &Address,
) -> Result<(), RecipientError> {
    caller.require_auth();

    let pending: PendingRecipient = env
        .storage()
        .instance()
        .get(&PENDING_RECIPIENT)
        .ok_or(RecipientError::NoPendingRecipient)?;

    if caller != &pending.proposer && caller != governor {
        return Err(RecipientError::Unauthorized);
    }

    env.storage().instance().remove(&PENDING_RECIPIENT);
    env.events().publish(
        (soroban_sdk::symbol_short!("rcpt_cncl"),),
        pending.recipient.clone(),
    );

    Ok(())
}

/// Activates a pending payout recipient once its rotation delay has elapsed.
///
/// Returns the newly active recipient. The delay is enforced against the
/// current ledger sequence: activating before `activates_at` fails with
/// `DelayNotElapsed`. On success the pending entry is cleared and the change is
/// evented.
pub fn activate_recipient(
    env: &Env,
    active: &mut Address,
) -> Result<Address, RecipientError> {
    let pending: PendingRecipient = env
        .storage()
        .instance()
        .get(&PENDING_RECIPIENT)
        .ok_or(RecipientError::NoPendingRecipient)?;

    if env.ledger().sequence() < pending.activates_at {
        return Err(RecipientError::DelayNotElapsed);
    }

    *active = pending.recipient.clone();
    env.storage().instance().remove(&PENDING_RECIPIENT);
    env.events().publish(
        (soroban_sdk::symbol_short!("rcpt_act"),),
        pending.recipient.clone(),
    );

    Ok(pending.recipient)
}

/// Returns the recipient that payouts must use.
///
/// Payouts always use the active recipient; a pending recipient is never
/// returned here, so fees accumulated during the rotation delay cannot be
/// redirected by an unactivated proposal.
pub fn payout_recipient(active: &Address) -> Address {
    active.clone()
}

/// Storage key for the pending payout recipient change.
const PENDING_RECIPIENT: soroban_sdk::Symbol = soroban_sdk::symbol_short!("pend_rcpt");

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn setup(env: &Env, state: TimelockActionState) -> (Vec<TimelockAction>, Address, Address) {
        let proposer = Address::generate(env);
      

/* … truncated 6605 chars — edit only what you need near the top … */
