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

use soroban_sdk::{contracterror, contracttype, Address, BytesN, Env, Vec};

/// Hard cap on the number of items accepted in a single batch.
pub const MAX_BATCH_SIZE: u32 = 32;

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
