//! Validator reward and slash accounting with auditable events (Issue #1418).
//!
//! Each bridge validator has an accounted balance. Reward credits increase
//! it and slashes debit it. Every change emits one event carrying the
//! validator, amount, reason category, affected epoch and the resulting
//! balance and lifetime totals, so an indexer can rebuild and reconcile each
//! validator's balance from events alone.
//!
//! Events are published only after the accounting change has been validated
//! and written. A rejected credit or slash publishes nothing.
//!
//! The event payloads are versioned with [`VALIDATOR_EVENT_SCHEMA_VERSION`]
//! and their XDR encoding is pinned by fixtures in `event-fixtures/`.

use soroban_sdk::{contracttype, Address, Env, Symbol};

use crate::BridgeError;

/// Schema version of [`EvtValidatorRewardCredited`] and
/// [`EvtValidatorSlashed`]. Bump on any field change and regenerate the
/// fixtures.
pub const VALIDATOR_EVENT_SCHEMA_VERSION: u32 = 1;

/// Length of one validator accounting epoch in ledger seconds.
pub const VALIDATOR_EPOCH_SECONDS: u64 = 86_400;

/// Why a validator was credited. Encoded as a `u32` on the wire.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ValidatorRewardReason {
    /// Share of bridge fees for relaying messages.
    RelayFee = 1,
    /// Reward for timely attestations / approvals.
    Attestation = 2,
    /// Reward for availability over an epoch.
    Uptime = 3,
}

/// Why a validator was slashed. Encoded as a `u32` on the wire.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ValidatorSlashReason {
    /// Conflicting signatures for the same message.
    DoubleSign = 1,
    /// Attested to an invalid or non-existent source transaction.
    InvalidAttestation = 2,
    /// Missed required participation in an epoch.
    Downtime = 3,
}

/// Accounted state for one validator.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorAccount {
    pub balance: i128,
    pub total_rewarded: i128,
    pub total_slashed: i128,
}

/// Emitted after a reward credit has been recorded.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtValidatorRewardCredited {
    pub schema_version: u32,
    pub validator: Address,
    pub amount: i128,
    pub reason: ValidatorRewardReason,
    pub epoch: u64,
    pub balance_after: i128,
    pub total_rewarded: i128,
}

/// Emitted after a slash debit has been recorded.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtValidatorSlashed {
    pub schema_version: u32,
    pub validator: Address,
    pub amount: i128,
    pub reason: ValidatorSlashReason,
    pub epoch: u64,
    pub balance_after: i128,
    pub total_slashed: i128,
}

/// Storage keys. Variant names are unique across the crate.
#[contracttype]
pub enum ValidatorAccountingKey {
    ValidatorAccount(Address),
}

pub fn current_epoch(env: &Env) -> u64 {
    env.ledger().timestamp() / VALIDATOR_EPOCH_SECONDS
}

pub fn get_account(env: &Env, validator: &Address) -> ValidatorAccount {
    env.storage()
        .persistent()
        .get(&ValidatorAccountingKey::ValidatorAccount(validator.clone()))
        .unwrap_or(ValidatorAccount {
            balance: 0,
            total_rewarded: 0,
            total_slashed: 0,
        })
}

fn save_account(env: &Env, validator: &Address, account: &ValidatorAccount) {
    env.storage().persistent().set(
        &ValidatorAccountingKey::ValidatorAccount(validator.clone()),
        account,
    );
}

fn validate(env: &Env, amount: i128, epoch: u64) -> Result<(), BridgeError> {
    if amount <= 0 {
        return Err(BridgeError::InvalidAmount);
    }
    if epoch > current_epoch(env) {
        return Err(BridgeError::InvalidValidatorEpoch);
    }
    Ok(())
}

/// Credit `amount` to `validator`. The caller has already checked that
/// `validator` is a member of the active validator set.
pub fn credit_reward(
    env: &Env,
    validator: Address,
    amount: i128,
    reason: ValidatorRewardReason,
    epoch: u64,
) -> Result<ValidatorAccount, BridgeError> {
    validate(env, amount, epoch)?;
    let mut account = get_account(env, &validator);
    account.balance = account
        .balance
        .checked_add(amount)
        .ok_or(BridgeError::InvalidAmount)?;
    account.total_rewarded = account
        .total_rewarded
        .checked_add(amount)
        .ok_or(BridgeError::InvalidAmount)?;
    save_account(env, &validator, &account);

    // Published only after the accounting write above succeeded.
    emit_reward_credited(
        env,
        EvtValidatorRewardCredited {
            schema_version: VALIDATOR_EVENT_SCHEMA_VERSION,
            validator,
            amount,
            reason,
            epoch,
            balance_after: account.balance,
            total_rewarded: account.total_rewarded,
        },
    );
    Ok(account)
}

/// Debit `amount` from `validator`. Fails without any change when the debit
/// would exceed the accounted balance.
pub fn slash(
    env: &Env,
    validator: Address,
    amount: i128,
    reason: ValidatorSlashReason,
    epoch: u64,
) -> Result<ValidatorAccount, BridgeError> {
    validate(env, amount, epoch)?;
    let mut account = get_account(env, &validator);
    if amount > account.balance {
        return Err(BridgeError::InsufficientValidatorBalance);
    }
    account.balance -= amount;
    account.total_slashed = account
        .total_slashed
        .checked_add(amount)
        .ok_or(BridgeError::InvalidAmount)?;
    save_account(env, &validator, &account);

    // Published only after the accounting write above succeeded.
    emit_slashed(
        env,
        EvtValidatorSlashed {
            schema_version: VALIDATOR_EVENT_SCHEMA_VERSION,
            validator,
            amount,
            reason,
            epoch,
            balance_after: account.balance,
            total_slashed: account.total_slashed,
        },
    );
    Ok(account)
}

/// Topics: `("bridge", "validator_reward_credited", validator, epoch)`.
pub fn emit_reward_credited(env: &Env, evt: EvtValidatorRewardCredited) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "bridge"),
            Symbol::new(env, "validator_reward_credited"),
            evt.validator.clone(),
            evt.epoch,
        ),
        evt,
    );
}

/// Topics: `("bridge", "validator_slashed", validator, epoch)`.
pub fn emit_slashed(env: &Env, evt: EvtValidatorSlashed) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "bridge"),
            Symbol::new(env, "validator_slashed"),
            evt.validator.clone(),
            evt.epoch,
        ),
        evt,
    );
}
