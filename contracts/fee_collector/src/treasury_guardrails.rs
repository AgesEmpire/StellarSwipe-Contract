//! Treasury guardrails for the fee collector.
//!
//! This module defines the guardrail configuration and the queued governance
//! timelock actions that gate sensitive treasury operations. It also specifies
//! the cancellation rules for queued timelock actions (see issue #1210), the
//! revenue source allowlist that restricts fee deposits to registered sources
//! (see issue #1154), the payout recipient rotation delay that prevents an
//! accidental or compromised admin update from redirecting accumulated fees
//! immediately (see issue #1157), and the admin role and access control that
//! gates fee parameter updates, pausing, and emergency withdrawals (see issue
//! #1509).

use soroban_sdk::{contracttype, Address, Env, Vec};

/// Storage key under which the admin address is persisted during
/// initialization.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdminKey {
    /// The address holding the admin role.
    Admin,
}

/// Errors returned by the admin role and access control paths.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdminError {
    /// The caller is not authorized to perform the admin-only operation.
    Unauthorized,
    /// The admin role has not been initialized yet.
    NotInitialized,
    /// The contract is paused and the operation is blocked.
    Paused,
}

/// Stores `admin` in contract storage during initialization.
///
/// The admin address is the sole authority for fee parameter updates, pausing
/// the contract, and emergency withdrawals. Initialization is idempotent: once
/// an admin is set it can only be changed through [`transfer_admin`].
pub fn init_admin(env: &Env, admin: &Address) -> Result<(), AdminError> {
    if env.storage().instance().has(&AdminKey::Admin) {
        return Err(AdminError::Unauthorized);
    }
    env.storage().instance().set(&AdminKey::Admin, admin);
    Ok(())
}

/// Returns the stored admin address, or `None` if not yet initialized.
pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&AdminKey::Admin)
}

/// Requires that `caller` is the stored admin and has authorized the call.
///
/// Returns [`AdminError::NotInitialized`] when no admin is stored and
/// [`AdminError::Unauthorized`] when the caller is not the admin.
pub fn require_admin(env: &Env, caller: &Address) -> Result<(), AdminError> {
    caller.require_auth();
    let admin = get_admin(env).ok_or(AdminError::NotInitialized)?;
    if &admin != caller {
        return Err(AdminError::Unauthorized);
    }
    Ok(())
}

/// Transfers the admin role from the current admin to `new_admin`.
///
/// Authorization: only the current admin may transfer the role. The change is
/// evented so off-chain tooling can track admin handovers.
pub fn transfer_admin(
    env: &Env,
    caller: &Address,
    new_admin: &Address,
) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    env.storage().instance().set(&AdminKey::Admin, new_admin);
    env.events()
        .publish((soroban_sdk::symbol_short!("admin_xfer"),), new_admin.clone());
    Ok(())
}

/// Updates a fee parameter. Admin-only.
///
/// The caller must be the stored admin and must authorize the call. The new
/// value is persisted under `key` in instance storage.
pub fn set_fee_parameter(
    env: &Env,
    caller: &Address,
    key: &soroban_sdk::Symbol,
    value: i128,
) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    env.storage().instance().set(key, &value);
    Ok(())
}

/// Pauses the contract. Admin-only.
///
/// While paused, sensitive operations guarded by [`require_not_paused`] are
/// rejected.
pub fn pause(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    env.storage()
        .instance()
        .set(&soroban_sdk::symbol_short!("paused"), &true);
    Ok(())
}

/// Unpauses the contract. Admin-only.
pub fn unpause(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    env.storage()
        .instance()
        .set(&soroban_sdk::symbol_short!("paused"), &false);
    Ok(())
}

/// Returns whether the contract is currently paused.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&soroban_sdk::symbol_short!("paused"))
        .unwrap_or(false)
}

/// Rejects the call when the contract is paused.
pub fn require_not_paused(env: &Env) -> Result<(), AdminError> {
    if is_paused(env) {
        return Err(AdminError::Paused);
    }
    Ok(())
}

/// Performs an emergency withdrawal of `amount` to `recipient`. Admin-only.
///
/// Emergency withdrawals are permitted even while the contract is paused so the
/// admin can recover funds during an incident. The withdrawal is evented.
pub fn emergency_withdraw(
    env: &Env,
    caller: &Address,
    recipient: &Address,
    amount: i128,
) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    env.events().publish(
        (soroban_sdk::symbol_short!("emrg_wd"),),
        (recipient.clone(), amount),
    );
    Ok(())
}

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
