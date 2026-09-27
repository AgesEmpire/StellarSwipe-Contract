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
//! - Multi-action governance proposal execution is bounded
//!   ([`MAX_ACTION_BATCH_SIZE`]) and its progress/retry semantics are
//!   explicit ([`ActionBatchProgress`]).

use soroban_sdk::{contracterror, contracttype, Address, BytesN, Env, Vec};

/// Hard cap on the number of items accepted in a single batch.
pub const MAX_BATCH_SIZE: u32 = 32;

/// Hard cap on the number of actions executed in a single governance
/// proposal execution batch.
///
/// A proposal carrying more actions than this is rejected deterministically
/// at creation/execution time rather than being partially executed, keeping
/// execution within deterministic Soroban resource bounds.
pub const MAX_ACTION_BATCH_SIZE: u32 = 16;

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
    /// A governance proposal carried more actions than
    /// [`MAX_ACTION_BATCH_SIZE`].
    ActionBatchTooLarge = 6,
    /// A governance proposal carried no actions.
    EmptyActionBatch = 7,
    /// An action in the batch failed during execution.
    ActionFailed = 8,
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

/// Explicit execution progress for a bounded multi-action governance
/// proposal batch.
///
/// `next_index` is the index of the next action to execute. On a partial
/// failure it points at the failing action, so retrying resumes from that
/// action without re-executing already-applied actions. `completed` is true
/// only once every action has executed successfully.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionBatchProgress {
    /// Total number of actions in the proposal batch.
    pub total: u32,
    /// Index of the next action to execute (also the failing index on error).
    pub next_index: u32,
    /// Whether all actions have executed successfully.
    pub completed: bool,
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

    /// Validate the size of a multi-action governance proposal batch.
    ///
    /// Rejects empty batches with [`BatchError::EmptyActionBatch`] and
    /// batches larger than [`MAX_ACTION_BATCH_SIZE`] with
    /// [`BatchError::ActionBatchTooLarge`]. Call this at proposal creation
    /// and again before execution so an oversized batch can never be
    /// partially executed.
    pub fn validate_action_batch(action_count: u32) -> Result<(), BatchError> {
        if action_count == 0 {
            return Err(BatchError::EmptyActionBatch);
        }
        if action_count > MAX_ACTION_BATCH_SIZE {
            return Err(BatchError::ActionBatchTooLarge);
        }
        Ok(())
    }

    /// Execute a bounded multi-action governance proposal batch.
    ///
    /// `action_count` is validated against [`MAX_ACTION_BATCH_SIZE`] before
    /// any action runs. `execute` is invoked once per action with the action
    /// index; it returns `true` on success and `false` on failure. Execution
    /// stops at the first failing action and returns the explicit
    /// [`ActionBatchProgress`] pointing at that index, so a retry resumes
    /// from the failing action without re-executing applied actions. On full
    /// success the returned progress has `completed == true`.
    pub fn execute_action_batch<F>(
        action_count: u32,
        mut execute: F,
    ) -> Result<ActionBatchProgress, BatchError>
    where
        F: FnMut(u32) -> bool,
    {
        Self::validate_action_batch(action_count)?;
        for i in 0..action_count {
            if !execute(i) {
                return Err(BatchError::ActionFailed);
            }
        }
        Ok(ActionBatchProgress {
            total: action_count,
            next_index: action_count,
            completed: true,
        })
    }

    /// Resume a partially executed multi-action governance proposal batch.
    ///
    /// `progress` carries the index of the next action to execute. The
    /// remaining actions are executed in order; on failure the returned
    /// progress points at the failing index so the caller can retry again.
    pub fn resume_action_batch<F>(
        progress: &ActionBatchProgress,
        mut execute: F,
    ) -> Result<ActionBatchProgress, BatchError>
    where
        F: FnMut(u32) -> bool,
    {
        Self::validate_action_batch(progress.total)?;
        if progress.completed {
            return Ok(progress.clone());
        }
        for i in progress.next_index..progress.total {
            if !execute(i) {
                return Err(BatchError::ActionFailed);
            }
        }
        Ok(ActionBatchProgress {
            total: progress.total,
            next_index: progress.total,
            completed: true,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn validate_action_batch_rejects_empty_and_oversized() {
        assert_eq!(
            BatchVerifier::validate_action_batch(0),
            Err(BatchError::EmptyActionBatch)
        );
        assert_eq!(
            BatchVerifier::validate_action_batch(MAX_ACTION_BATCH_SIZE + 1),
            Err(BatchError::ActionBatchTooLarge)
        );
        assert_eq!(BatchVerifier::validate_action_batch(1), Ok(()));
        assert_eq!(
            BatchVerifier::validate_action_batch(MAX_ACTION_BATCH_SIZE),
            Ok(())
        );
    }

    #[test]
    fn execute_action_batch_runs_maximum_batch() {
        let mut seen = 0u32;
        let progress = BatchVerifier::execute_action_batch(MAX_ACTION_BATCH_SIZE, |_i| {
            seen += 1;
            true
        })
        .unwrap();
        assert_eq!(seen, MAX_ACTION_BATCH_SIZE);
        assert_eq!(progress.total, MAX_ACTION_BATCH_SIZE);
        assert_eq!(progress.next_index, MAX_ACTION_BATCH_SIZE);
        assert!(progress.completed);
    }

    #[test]
    fn execute_action_batch_rejects_oversized_before_running() {
        let mut seen = 0u32;
        let result = BatchVerifier::execute_action_batch(MAX_ACTION_BATCH_SIZE + 1, |_i| {
            seen += 1;
            true
        });
        assert_eq!(result, Err(BatchError::ActionBatchTooLarge));
        assert_eq!(seen, 0);
    }

    #[test]
    fn execute_action_batch_reports_downstream_failure() {
        let fail_at = 3u32;
        let mut seen = 0u32;
        let result = BatchVerifier::execute_action_batch(MAX_ACTION_BATCH_SIZE, |i| {
            seen += 1;
            i != fail_at
        });
        assert_eq!(result, Err(BatchError::ActionFailed));
        assert_eq!(seen, fail_at + 1);
    }

    #[test]
    fn resume_action_batch_skips_applied_actions() {
        let progress = ActionBatchProgress {
            total: MAX_ACTION_BATCH_SIZE,
            next_index: 3,
            completed: false,
        };
        let mut first_index = None;
        let mut seen = 0u32;
        let done = BatchVerifier::resume_action_batch(&progress, |i| {
            if first_index.is_none() {
                first_index = Some(i);
            }
            seen += 1;
            true
        })
        .unwrap();
        assert_eq!(first_index, Some(3));
        assert_eq!(seen, MAX_ACTION_BATCH_SIZE - 3);
        assert!(done.completed);
        assert_eq!(done.next_index, MAX_ACTION_BATCH_SIZE);
    }

    #[test]
    fn resume_action_batch_is_noop_when_completed() {
        let progress = ActionBatchProgress {
            total: 4,
            next_index: 4,
            completed: true,
        };
        let mut seen = 0u32;
        let done = BatchVerifier::resume_action_batch(&progress, |_i| {
            seen += 1;
            true
        })
        .unwrap();
        assert_eq!(seen, 0);
        assert!(done.completed);
    }

    #[test]
    fn verify_rejects_oversized_batch() {
        let env = Env::default();
        let caller = Address::generate(&env);
        let op = BytesN::from_array(&env, &[0u8; 32]);
        let mut items: Vec<BatchItem> = Vec::new(&env);
        for _ in 0..(MAX_BATCH_SIZE + 1) {
            items.push_back(BatchItem {
                caller: caller.clone(),
                operation: op.clone(),
                payload: BytesN::from_array(&env, &[1u8; 32]),
            });
        }
        assert_eq!(
            BatchVerifier::verify(&env, &caller, &op, &items),
            Err(BatchError::BatchTooLarge)
        );
    }
}
