//! Bounded recovery queue for failed bridge deliveries (Issue #1415).
//!
//! * Only failures classified as [`FailureClass::Transient`] are queued.
//!   Permanent failures are terminal and never enter the queue.
//! * The queue holds at most [`MAX_RECOVERY_QUEUE_LEN`] transfers and never
//!   holds the same transfer twice, so storage cannot grow without bound.
//! * [`process_queue`] takes a caller-supplied maximum that must be within
//!   `1..=MAX_RECOVERY_BATCH`. Processing only rewrites transfer state back
//!   to `ReadyToExecute`; it performs no external or cross-contract calls, so
//!   one transaction can never fan out into repeated delivery attempts. Each
//!   re-execution is a separate `execute_*` transaction.
//! * A retry keeps the transfer's identity: id, source tx hash, source nonce,
//!   amount, recipient and expiry are untouched, so the signed message digest
//!   is the same before and after the retry.
//! * Every retry counts against the attempt policy. Once a transfer has used
//!   its configured attempts, a further transient failure is recorded as
//!   permanent and the transfer leaves the queue for good.

use soroban_sdk::{contracttype, Env, String, Symbol, Vec};

use crate::{BridgeError, BridgeTransfer, TransferStatus};

/// Protocol cap on the number of queued transfers.
pub const MAX_RECOVERY_QUEUE_LEN: u32 = 64;
/// Protocol cap on transfers processed in one `process_recovery_queue` call.
pub const MAX_RECOVERY_BATCH: u32 = 10;
/// Retry attempts allowed per transfer when no policy has been configured.
pub const DEFAULT_MAX_DELIVERY_ATTEMPTS: u32 = 3;
/// Upper bound an admin may configure for the attempt policy.
pub const MAX_DELIVERY_ATTEMPTS_CAP: u32 = 10;

/// Classification of a delivery failure.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FailureClass {
    /// Temporary condition (e.g. RPC timeout, congestion); a retry may succeed.
    Transient = 1,
    /// The delivery can never succeed; terminal.
    Permanent = 2,
}

/// Storage keys. Variant names are unique across the crate.
#[contracttype]
pub enum RecoveryKey {
    /// `Vec<u64>` of queued transfer ids, oldest first.
    RecoveryQueue,
    /// Admin-configured maximum retry attempts per transfer.
    RecoveryMaxAttempts,
    /// Retry attempts already used by a transfer.
    RecoveryAttempts(u64),
}

pub fn get_queue(env: &Env) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&RecoveryKey::RecoveryQueue)
        .unwrap_or_else(|| Vec::new(env))
}

fn save_queue(env: &Env, queue: &Vec<u64>) {
    env.storage()
        .persistent()
        .set(&RecoveryKey::RecoveryQueue, queue);
}

pub fn get_max_attempts(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&RecoveryKey::RecoveryMaxAttempts)
        .unwrap_or(DEFAULT_MAX_DELIVERY_ATTEMPTS)
}

pub fn set_max_attempts(env: &Env, max_attempts: u32) -> Result<(), BridgeError> {
    if max_attempts == 0 || max_attempts > MAX_DELIVERY_ATTEMPTS_CAP {
        return Err(BridgeError::InvalidAmount);
    }
    env.storage()
        .persistent()
        .set(&RecoveryKey::RecoveryMaxAttempts, &max_attempts);
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "bridge"),
            Symbol::new(env, "recovery_policy_set"),
        ),
        max_attempts,
    );
    Ok(())
}

pub fn get_attempts(env: &Env, transfer_id: u64) -> u32 {
    env.storage()
        .persistent()
        .get(&RecoveryKey::RecoveryAttempts(transfer_id))
        .unwrap_or(0)
}

fn record_attempt(env: &Env, transfer_id: u64) -> u32 {
    let attempts = get_attempts(env, transfer_id).saturating_add(1);
    env.storage()
        .persistent()
        .set(&RecoveryKey::RecoveryAttempts(transfer_id), &attempts);
    attempts
}

fn queue_position(queue: &Vec<u64>, transfer_id: u64) -> Option<u32> {
    for i in 0..queue.len() {
        if queue.get(i) == Some(transfer_id) {
            return Some(i);
        }
    }
    None
}

/// Remove `transfer_id` from the queue if present. Returns whether it was queued.
pub fn dequeue(env: &Env, transfer_id: u64) -> bool {
    let mut queue = get_queue(env);
    match queue_position(&queue, transfer_id) {
        Some(i) => {
            queue.remove(i);
            save_queue(env, &queue);
            true
        }
        None => false,
    }
}

/// Apply a classified failure to `transfer` (already validated as failable by
/// the caller). Returns the transfer's resulting `retryable` flag.
///
/// The queue-capacity check runs before anything is written, so a full queue
/// leaves the transfer and the queue unchanged.
pub fn record_failure(
    env: &Env,
    transfer: &mut BridgeTransfer,
    reason: String,
    class: FailureClass,
) -> Result<bool, BridgeError> {
    let mut queue = get_queue(env);
    let already_queued = queue_position(&queue, transfer.id);
    let attempts = get_attempts(env, transfer.id);
    let max_attempts = get_max_attempts(env);

    let retryable = class == FailureClass::Transient && attempts < max_attempts;

    if retryable && already_queued.is_none() && queue.len() >= MAX_RECOVERY_QUEUE_LEN {
        return Err(BridgeError::RecoveryQueueFull);
    }

    transfer.status = TransferStatus::Failed;
    transfer.failure_reason = Some(reason.clone());
    transfer.retryable = retryable;

    if retryable {
        if already_queued.is_none() {
            queue.push_back(transfer.id);
            save_queue(env, &queue);
        }
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "recovery_queued"), transfer.id),
            (reason, attempts, max_attempts),
        );
    } else {
        if let Some(i) = already_queued {
            queue.remove(i);
            save_queue(env, &queue);
        }
        let event_name = if class == FailureClass::Transient {
            // Transient, but the attempt policy is used up.
            "recovery_exhausted"
        } else {
            "transfer_failed_permanent"
        };
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, event_name), transfer.id),
            (reason, attempts, max_attempts),
        );
    }
    Ok(retryable)
}

/// Re-arm a failed, retryable transfer for execution and count the attempt.
/// Identity fields are left untouched. Shared by the queue processor and
/// the manual `retry_transfer` entrypoint so both respect the attempt policy.
pub fn begin_retry(env: &Env, transfer: &mut BridgeTransfer) -> u32 {
    dequeue(env, transfer.id);
    arm_retry(env, transfer)
}

fn arm_retry(env: &Env, transfer: &mut BridgeTransfer) -> u32 {
    let attempts = record_attempt(env, transfer.id);
    transfer.status = TransferStatus::ReadyToExecute;
    transfer.failure_reason = None;
    transfer.retryable = false;
    attempts
}

/// Process up to `max_items` queued transfers, oldest first.
///
/// `load`/`store` abstract the transfer storage so this module stays free of
/// the contract's storage layout. `is_expired` lets the caller drop transfers
/// whose validity window has ended instead of re-arming them.
///
/// Returns the number of transfers re-armed for execution.
pub fn process_queue(
    env: &Env,
    max_items: u32,
    load: impl Fn(&Env, u64) -> Option<BridgeTransfer>,
    store: impl Fn(&Env, &BridgeTransfer),
    is_expired: impl Fn(&Env, &BridgeTransfer) -> bool,
) -> Result<u32, BridgeError> {
    if max_items == 0 || max_items > MAX_RECOVERY_BATCH {
        return Err(BridgeError::RecoveryBatchLimitExceeded);
    }

    let mut queue = get_queue(env);
    let mut handled: u32 = 0;
    let mut retried: u32 = 0;

    while handled < max_items && !queue.is_empty() {
        let transfer_id = match queue.pop_front() {
            Some(id) => id,
            None => break,
        };
        handled += 1;

        let mut transfer = match load(env, transfer_id) {
            Some(t) => t,
            None => continue,
        };
        // Stale entry: the transfer was retried manually, completed or
        // reclassified since it was queued.
        if transfer.status != TransferStatus::Failed || !transfer.retryable {
            continue;
        }
        if is_expired(env, &transfer) {
            transfer.retryable = false;
            store(env, &transfer);
            #[allow(deprecated)]
            env.events().publish(
                (Symbol::new(env, "recovery_dropped_expired"), transfer_id),
                env.ledger().timestamp(),
            );
            continue;
        }

        // The entry was already popped from the local queue, which is
        // persisted once after the loop.
        let attempts = arm_retry(env, &mut transfer);
        store(env, &transfer);
        retried += 1;

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "transfer_recovery_retry"), transfer_id),
            (attempts, get_max_attempts(env), env.ledger().timestamp()),
        );
    }

    save_queue(env, &queue);
    Ok(retried)
}
