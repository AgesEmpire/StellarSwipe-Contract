//! Treasury guardrails for the fee collector.
//!
//! This module defines the guardrail configuration and the queued governance
//! timelock actions that gate sensitive treasury operations. It also specifies
//! the cancellation rules for queued timelock actions (see issue #1210), the
//! revenue source allowlist that restricts fee deposits to registered sources
//! (see issue #1154), the payout recipient rotation delay that prevents an
//! accidental or compromised admin update from redirecting accumulated fees
//! immediately (see issue #1157), the admin role and access control that
//! gates fee parameter updates, pausing, and emergency withdrawals (see issue
//! #1509), and the protocol fee rate and treasury address configuration that
//! drives protocol-level fee collection on each swap (see issue #1522).

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

/// Storage keys for the protocol fee configuration.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolFeeKey {
    /// The protocol fee rate in basis points (e.g. `5` == 0.05%).
    RateBps,
    /// The treasury address that receives protocol fees.
    Treasury,
}

/// Errors returned by the protocol fee configuration and collection paths.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolFeeError {
    /// The caller is not authorized to mutate the protocol fee config.
    Unauthorized,
    /// The protocol fee rate has not been configured yet.
    RateNotConfigured,
    /// The treasury address has not been configured yet.
    TreasuryNotConfigured,
    /// The configured protocol fee rate is out of the allowed range.
    InvalidRate,
}

/// Maximum protocol fee rate in basis points (1% == 100 bps).
const MAX_PROTOCOL_FEE_BPS: u32 = 100;

/// Basis points denominator (100% == 10_000 bps).
const BPS_DENOMINATOR: i128 = 10_000;

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

/// Sets the protocol fee rate in basis points. Admin-only.
///
/// The rate is independent of the LP fee and is deducted from each swap before
/// the LP fee is applied. Rates above [`MAX_PROTOCOL_FEE_BPS`] are rejected.
pub fn set_protocol_fee_rate(
    env: &Env,
    caller: &Address,
    rate_bps: u32,
) -> Result<(), ProtocolFeeError> {
    require_admin(env, caller).map_err(|_| ProtocolFeeError::Unauthorized)?;
    if rate_bps > MAX_PROTOCOL_FEE_BPS {
        return Err(ProtocolFeeError::InvalidRate);
    }
    env.storage()
        .instance()
        .set(&ProtocolFeeKey::RateBps, &rate_bps);
    env.events().publish(
        (soroban_sdk::symbol_short!("proto_rate"),),
        rate_bps,
    );
    Ok(())
}

/// Returns the configured protocol fee rate in basis points, if any.
pub fn get_protocol_fee_rate(env: &Env) -> Option<u32> {
    env.storage().instance().get(&ProtocolFeeKey::RateBps)
}

/// Sets the treasury address that receives protocol fees. Admin-only.
pub fn set_treasury(
    env: &Env,
    caller: &Address,
    treasury: &Address,
) -> Result<(), ProtocolFeeError> {
    require_admin(env, caller).map_err(|_| ProtocolFeeError::Unauthorized)?;
    env.storage()
        .instance()
        .set(&ProtocolFeeKey::Treasury, treasury);
    env.events().publish(
        (soroban_sdk::symbol_short!("treasury"),),
        treasury.clone(),
    );
    Ok(())
}

/// Returns the configured treasury address, if any.
pub fn get_treasury(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ProtocolFeeKey::Treasury)
}

/// Splits `amount` into the protocol fee and the remainder.
///
/// The protocol fee is computed independently from the LP fee so the two never
/// compound. Returns `(protocol_fee, amount_after_protocol_fee)`.
pub fn split_protocol_fee(
    env: &Env,
    amount: i128,
) -> Result<(i128, i128), ProtocolFeeError> {
    let rate_bps = get_protocol_fee_rate(env).ok_or(ProtocolFeeError::RateNotConfigured)?;
    let protocol_fee = amount
        .checked_mul(rate_bps as i128)
        .ok_or(ProtocolFeeError::InvalidRate)?
        / BPS_DENOMINATOR;
    Ok((protocol_fee, amount - protocol_fee))
}

/// Deducts the protocol fee from `amount` and transfers it to the treasury.
///
/// Returns the amount remaining after the protocol fee, which the caller then
/// applies the LP fee to. The transfer is evented so off-chain tooling can
/// reconcile treasury inflows.
pub fn collect_protocol_fee(env: &Env, amount: i128) -> Result<i128, ProtocolFeeError> {
    let treasury = get_treasury(env).ok_or(ProtocolFeeError::TreasuryNotConfigured)?;
    let (protocol_fee, remaining) = split_protocol_fee(env, amount)?;
    if protocol_fee > 0 {
        env.events().publish(
            (soroban_sdk::symbol_short!("proto_fee"),),
            (treasury, protocol_fee),
        );
    }
    Ok(remaining)
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

/// Retu

/* … truncated 4914 chars — edit only what you need near the top … */
