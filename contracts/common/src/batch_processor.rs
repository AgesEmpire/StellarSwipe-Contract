//! Bounded batch authorization verification.
//!
//! Provides a verifier for repeated authorization checks where batching
//! reduces resource use, while preserving per-item failure reporting and
//! atomicity requirements.
//!
//! Guarantees:
//! - The batch size is explicitly capped ([`MAX_BATCH_SIZE`]). Oversized
//!   batches are rejected deterministically rather than silently truncated.
//! - Each item is bound to its intended operation and caller, so an item
//!   cannot be replayed against a different operation or caller.
//! - Invalid items produce explicit per-item failure results and are never
//!   silently skipped.
//! - Cross-contract invocation depth is explicitly capped
//!   ([`MAX_INVOCATION_DEPTH`]) and re-entrant/cyclic call patterns are
//!   rejected before any irreversible effects occur.
//! - Escrow liabilities are reconciled against the tokens actually held for
//!   escrowed user positions ([`EscrowLedger`]).

use soroban_sdk::{contracterror, contracttype, Address, BytesN, Env, Vec};

/// Hard cap on the number of items accepted in a single batch.
pub const MAX_BATCH_SIZE: u32 = 32;

/// Hard cap on the supported cross-contract invocation depth.
///
/// A call chain deeper than this is rejected deterministically. This bounds
/// recursion so cyclic cross-contract patterns cannot recurse until resource
/// exhaustion.
pub const MAX_INVOCATION_DEPTH: u32 = 8;

/// Errors surfaced by the batch verifier.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BatchError {
    /// The supplied batch exceeded [`MAX_BATCH_SIZE`].
    BatchTooLarge = 1,
    /// The batch was empty.
    EmptyBatch = 2,
    /// An item's binding did not match the expected operation/caller.
    BindingMismatch = 3,
    /// A cross-contract invocation cycle was detected.
    InvocationCycle = 4,
    /// The cross-contract invocation depth exceeded [`MAX_INVOCATION_DEPTH`].
    InvocationDepthExceeded = 5,
    /// Escrow liabilities did not reconcile against held token balances.
    EscrowImbalance = 6,
}

/// A single authorization check bound to its intended operation and caller.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchItem {
    /// The caller the authorization is bound to.
    pub caller: Address,
    /// The operation identifier the authorization is bound to.
    pub operation: BytesN<32>,
    /// The payload digest being authorized.
    pub payload: BytesN<32>,
}

/// Per-item verification outcome. Every item yields exactly one result so
/// invalid items are reported explicitly and never silently skipped.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchItemResult {
    /// Index of the item within the submitted batch.
    pub index: u32,
    /// Whether the item passed verification.
    pub ok: bool,
    /// Error code when `ok` is false; `0` when the item passed.
    pub error: u32,
}

/// Bounded batch authorization verifier.
pub struct BatchVerifier;

impl BatchVerifier {
    /// Verify a batch of authorization items against the expected operation
    /// and caller.
    ///
    /// The batch size is capped at [`MAX_BATCH_SIZE`]; oversized batches are
    /// rejected with [`BatchError::BatchTooLarge`] and empty batches with
    /// [`BatchError::EmptyBatch`]. Each item is checked against `expected_op`
    /// and `expected_caller`; mismatches are reported per item via
    /// [`BatchItemResult`] rather than being skipped.
    pub fn verify(
        env: &Env,
        expected_caller: &Address,
        expected_op: &BytesN<32>,
        items: &Vec<BatchItem>,
    ) -> Result<Vec<BatchItemResult>, BatchError> {
        let len = items.len();
        if len == 0 {
            return Err(BatchError::EmptyBatch);
        }
        if len > MAX_BATCH_SIZE {
            return Err(BatchError::BatchTooLarge);
        }

        let mut results: Vec<BatchItemResult> = Vec::new(env);
        for i in 0..len {
            let item = items.get_unchecked(i);
            let bound = item.caller == *expected_caller && item.operation == *expected_op;
            let error = if bound {
                0
            } else {
                BatchError::BindingMismatch as u32
            };
            results.push_back(BatchItemResult {
                index: i,
                ok: bound,
                error,
            });
        }
        Ok(results)
    }

    /// Verify a batch and require every item to pass. Returns
    /// [`BatchError::BindingMismatch`] if any item failed, preserving the
    /// atomicity requirement for callers that need all-or-nothing semantics.
    pub fn verify_all(
        env: &Env,
        expected_caller: &Address,
        expected_op: &BytesN<32>,
        items: &Vec<BatchItem>,
    ) -> Result<(), BatchError> {
        let results = Self::verify(env, expected_caller, expected_op, items)?;
        for i in 0..results.len() {
            if !results.get_unchecked(i).ok {
                return Err(BatchError::BindingMismatch);
            }
        }
        Ok(())
    }
}

/// Reusable guard enforcing the cross-contract invocation depth and cycle
/// policy.
///
/// Call sites push the contract identifier they are about to invoke onto the
/// `stack` before performing the call. The guard rejects the invocation when
/// the identifier is already present on the stack (a direct `A->A` or
/// indirect `A->B->A` cycle) or when the resulting depth would exceed
/// [`MAX_INVOCATION_DEPTH`]. Because the guard runs before the call is made,
/// rejected cycles fail before any irreversible effects (state writes,
/// transfers, events) occur.
pub struct InvocationGuard;

impl InvocationGuard {
    /// Check whether invoking `contract_id` from the current `stack` is
    /// permitted.
    ///
    /// Returns [`BatchError::InvocationCycle`] if `contract_id` is already on
    /// the stack, or [`BatchError::InvocationDepthExceeded`] if pushing it
    /// would exceed [`MAX_INVOCATION_DEPTH`]. On success the caller may push
    /// `contract_id` onto the stack and proceed with the invocation.
    pub fn check(stack: &Vec<BytesN<32>>, contract_id: &BytesN<32>) -> Result<(), BatchError> {
        let len = stack.len();
        for i in 0..len {
            if stack.get_unchecked(i) == *contract_id {
                return Err(BatchError::InvocationCycle);
            }
        }
        if len >= MAX_INVOCATION_DEPTH {
            return Err(BatchError::InvocationDepthExceeded);
        }
        Ok(())
    }

    /// Check and record an invocation in one step, returning the updated
    /// stack. Rejected invocations return an error and leave the caller's
    /// stack untouched, so no irreversible effects are performed.
    pub fn enter(
        env: &Env,
        stack: &Vec<BytesN<32>>,
        contract_id: &BytesN<32>,
    ) -> Result<Vec<BytesN<32>>, BatchError> {
        Self::check(stack, contract_id)?;
        let mut next = stack.clone();
        next.push_back(contract_id.clone());
        let _ = env;
        Ok(next)
    }
}

/// A single escrowed user position tracked by the ledger.
///
/// `liability` is the amount the contract owes the user for this position;
/// `held` is the amount of the token actually held on the contract's behalf
/// for this position. The invariant requires `held >= liability` for every
/// position so escrowed funds are always fully backed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowPosition {
    /// Identifier of the escrowed position.
    pub id: BytesN<32>,
    /// Token the position is denominated in.
    pub token: Address,
    /// Amount owed to the user for this position.
    pub liability: i128,
    /// Amount of `token` actually held for this position.
    pub held: i128,
}

/// Diagnostic report produced by [`EscrowLedger::reconcile`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowReconciliation {
    /// Total liabilities across all escrowed positions.
    pub total_liability: i128,
    /// Total tokens held across all escrowed positions.
    pub total_held: i128,
    /// Whether the invariant holds (`total_held >= total_liability` and every
    /// individual position is fully backed).
    pub balanced: bool,
    /// Index of the first position that violated the invariant, if any.
    pub first_violation: u32,
}

/// Escrow liability ledger enforcing that escrowed user positions are always
/// backed by the tokens actually held.
///
/// Every escrow creation and release path must go through [`EscrowLedger`]
/// so the invariant is preserved:
/// - [`EscrowLedger::create`] records a new position only when the tokens
///   backing it are already held.
/// - [`EscrowLedger::release`] reduces both the liability and the held amount
///   by the same released quantity, so a partial settlement, cancellation, or
///   failed downstream execution cannot leave liabilities exceeding held
///   tokens.
/// - [`EscrowLedger::reconcile`] is the checkable diagnostic entrypoint that
///   verifies the invariant across all positions.
pub struct EscrowLedger;

impl EscrowLedger {
    /// Record a newly escrowed position.
    ///
    /// The position is only accepted when the tokens backing it are already
    /// held (`held >= liability`), preserving the invariant at creation time.
    pub fn create(
        env: &Env,
        positions: &Vec<EscrowPosition>,
        position: &EscrowPosition,
    ) -> Result<Vec<EscrowPosition>, BatchError> {
        if position.liability < 0 || position.held < position.liability {
            return Err(BatchError::EscrowImbalance);
        }
        let mut next = positions.clone();
        next.push_back(position.clone());
        let _ = env;
        Ok(next)
    }

    /// Release `amount` of a position's liability and held tokens together.
    ///
    /// Used by settlement, cancellation, and failed downstream execution
    /// paths. Both `liability` and `held` are reduced by the same `amount`,
    /// so the invariant is preserved for partial releases. Releasing more
    /// than the position's liability is rejected.
    pub fn release(
        env: &Env,
        positions: &Vec<EscrowPosition>,
        id: &BytesN<32>,
        amount: i128,
    ) -> Result<Vec<EscrowPosition>, BatchError> {
        if amount < 0 {
            return Err(BatchError::EscrowImbalance);
        }
        let len = positions.len();
        let mut next: Vec<EscrowPosition> = Vec::new(env);
        let mut found = false;
        for i in 0..len {
            let mut position = positions.get_unchecked(i);
            if position.id == *id {
                found = true;
                if amount > position.liability {
                    return Err(BatchError::EscrowImbalance);
                }
                position.liability -= amount;
                position.held -= amount;
                if position.held < position.liability {
                    return Err(BatchError::EscrowImbalance);
                }
            }
            next.push_back(position);
        }
        if !found {
            return Err(BatchError::EscrowImbalance);
        }
        Ok(next)
    }

    /// Checkable diagnostic entrypoint reconciling escrow liabilities against
    /// the tokens actually held for escrowed user positions.
    ///
    /// Returns an [`EscrowReconciliation`] report. `balanced` is true only
    /// when every position is fully backed and the aggregate held amount
    /// covers the aggregate liability. `first_violation` identifies the first
    /// offending position index, or `0` when balanced.
    pub fn reconcile(positions: &Vec<EscrowPosition>) -> EscrowReconciliation {
        let len = positions.len();
        let mut total_liability: i128 = 0;
        let mut total_held: i128 = 0;
        let mut balanced = true;
        let mut first_violation: u32 = 0;
        for i in 0..len {
            let position = positions.get_unchecked(i);
            total_liability += position.liability;
            total_held += position.held;
            if position.held < position.liability {
                if balanced {
                    first_violation = i;
                }
                balanced = false;
            }
        }
        if total_held < total_liability {
            balanced = false;
        }
        EscrowReconciliation {
            total_liability,
            total_held,
            balanced,
            first_violation,
        }
    }

    /// Enforce the invariant, returning [`BatchError::EscrowImbalance`] when
    /// liabilities are not fully backed by held tokens.
    pub fn assert_balanced(positions: &Vec<EscrowPosition>) -> Result<(), BatchError> {
        if Self::reconcile(positions).balanced {
            Ok(())
        } else {
            Err(BatchError::EscrowImbalance)
        }
    }
}
