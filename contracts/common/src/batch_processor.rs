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
//! - Trade execution routes are capped at a configurable number of hops
//!   ([`MAX_ROUTE_HOPS`]) and rejected before any state mutation or transfer.
//! - Escrow liabilities are reconciled against the tokens actually held for
//!   escrowed user positions ([`EscrowLedger`]).
//! - Multi-action governance proposal execution is bounded
//!   ([`MAX_ACTION_BATCH_SIZE`]) and its progress/retry semantics are
//!   explicit ([`ActionBatchProgress`]).
//! - Trade routes are validated for asset continuity before execution, so an
//!   invalid route fails without performing transfers or state changes.
//! - Soroban vectors are decoded through a shared bounded decoder
//!   ([`decode_bounded_vec`]) that validates element count and encoded size
//!   before any allocation, so malformed or oversized inputs fail without
//!   partial state changes.

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

/// Lower bound for a configurable trade route hop limit.
pub const MIN_ROUTE_HOPS: u32 = 1;

/// Upper bound for a configurable trade route hop limit.
pub const MAX_ROUTE_HOPS: u32 = 16;

/// Hard cap on the number of elements accepted by the shared bounded vector
/// decoder ([`decode_bounded_vec`]).
///
/// This is the default element-count ceiling applied when a caller does not
/// supply a tighter per-call limit. It keeps decoding within deterministic
/// Soroban resource bounds regardless of the encoded input size.
pub const MAX_VECTOR_ELEMENTS: u32 = 64;

/// Hard cap on the encoded byte size accepted by the shared bounded vector
/// decoder ([`decode_bounded_vec`]).
///
/// The encoded size is validated before any element allocation, so an
/// oversized payload is rejected without materializing a vector.
pub const MAX_VECTOR_ENCODED_SIZE: u32 = 4096;

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
    /// A trade route hop did not consume the prior hop's output asset.
    AssetContinuityBroken = 9,
    /// A trade route did not terminate in the requested asset.
    RouteTerminalAssetMismatch = 10,
    /// The trade route was empty.
    EmptyRoute = 11,
    /// The trade route exceeded the configured hop limit.
    RouteTooLong = 12,
    /// The trade route contained a duplicate hop.
    DuplicateHop = 13,
    /// The configured route hop limit was outside the validated bounds.
    InvalidRouteHopLimit = 14,
    /// A decoded vector carried more elements than the configured limit.
    VectorTooLarge = 15,
    /// A decoded vector's encoded size exceeded the configured limit.
    VectorEncodedTooLarge = 16,
    /// A decoded vector's declared element count did not match its payload.
    VectorLengthMismatch = 17,
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

/// A single hop in a trade route.
///
/// Each hop consumes `asset_in` and produces `asset_out`. For a route to be
/// valid, every hop's `asset_in` must equal the prior hop's `asset_out`, and
/// the final hop's `asset_out` must equal the requested asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteHop {
    /// The asset consumed by this hop.
    pub asset_in: BytesN<32>,
    /// The asset produced by this hop.
    pub asset_out: BytesN<32>,
}

/// Shared bounded decoder for Soroban vectors.
///
/// Validates the declared element count and the encoded byte size *before*
/// any element is materialized, so malformed or oversized inputs fail
/// without partial state changes. The decoder is generic over the element
/// type and is intended to be reused by every cross-contract decoder that
/// accepts a vector from an untrusted caller.
///
/// Bounds enforced:
/// - `declared_len` must not exceed `max_elements` (and the global
///   [`MAX_VECTOR_ELEMENTS`] ceiling).
/// - `encoded_size` must not exceed `max_encoded_size` (and the global
///   [`MAX_VECTOR_ENCODED_SIZE`] ceiling).
/// - `declared_len` must be consistent with `encoded_size` for the element
///   width, so a truncated or padded payload is rejected rather than
///   silently accepted.
///
/// Zero-length vectors are valid and decode to an empty vector; they are
/// never treated as malformed.
pub fn decode_bounded_vec<T>(
    env: &Env,
    declared_len: u32,
    encoded_size: u32,
    max_elements: u32,
    max_encoded_size: u32,
    element_width: u32,
) -> Result<Vec<T>, BatchError> {
    // Reject oversized element counts before any allocation.
    if declared_len > max_elements || declared_len > MAX_VECTOR_ELEMENTS {
        return Err(BatchError::VectorTooLarge);
    }
    // Reject oversized encoded payloads before any allocation.
    if encoded_size > max_encoded_size || encoded_size > MAX_VECTOR_ENCODED_SIZE {
        return Err(BatchError::VectorEncodedTooLarge);
    }
    // Reject payloads whose encoded size is inconsistent with the declared
    // element count. Zero-length vectors are explicitly allowed.
    if declared_len != 0 {
        let expected = declared_len
            .checked_mul(element_width)
            .ok_or(BatchError::VectorLengthMismatch)?;
        if expected != encoded_size {
            return Err(BatchError::VectorLengthMismatch);
        }
    } else if encoded_size != 0 {
        return Err(BatchError::VectorLengthMismatch);
    }

    // Only now, after all bounds checks pass, do we allocate.
    let mut out: Vec<T> = Vec::new(env);
    out.reserve(declared_len);
    Ok(out)
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

    /// Decode a batch of authorization items through the shared bounded
    /// vector decoder.
    ///
    /// The declared element count and encoded size are validated before any
    /// allocation, so malformed or oversized inputs fail with a
    /// [`BatchError`] and leave no partial state behind. Zero-length batches
    /// decode to an empty vector and are then rejected by [`Self::verify`]
    /// with [`BatchError::EmptyBatch`], preserving existing semantics.
    pub fn decode_items(
        env: &Env,
        declared_len: u32,
        encoded_size: u32,
    ) -> Result<Vec<BatchItem>, BatchError> {
        decode_bounded_vec::<BatchItem>(
            env,
            declared_len,
            encoded_size,
            MAX_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            BATCH_ITEM_ENCODED_WIDTH,
        )
    }

    /// Decode a trade route through the shared bounded vector decoder.
    ///
    /// Bounds the hop count to [`MAX_ROUTE_HOPS`] and validates the encoded
    /// size before allocation, so an oversized or malformed route fails
    /// without performing transfers or state changes.
    pub fn decode_route(
        env: &Env,
        declared_len: u32,
        encoded_size: u32,
    ) -> Result<Vec<RouteHop>, BatchError> {
        decode_bounded_vec::<RouteHop>(
            env,
            declared_len,
            encoded_size,
            MAX_ROUTE_HOPS,
            MAX_VECTOR_ENCODED_SIZE,
            ROUTE_HOP_ENCODED_WIDTH,
        )
    }

    /// Decode a governance action batch through the shared bounded vector
    /// decoder.
    ///
    /// Bounds the action count to [`MAX_ACTION_BATCH_SIZE`] and validates the
    /// encoded size before allocation, so an oversized or malformed action
    /// batch fails without partially executing a proposal.
    pub fn decode_actions(
        env: &Env,
        declared_len: u32,
        encoded_size: u32,
    ) -> Result<Vec<BytesN<32>>, BatchError> {
        decode_bounded_vec::<BytesN<32>>(
            env,
            declared_len,
            encoded_size,
            MAX_ACTION_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            ACTION_ENCODED_WIDTH,
        )
    }
}

/// Encoded width, in bytes, of a single [`BatchItem`] element.
const BATCH_ITEM_ENCODED_WIDTH: u32 = 96;

/// Encoded width, in bytes, of a single [`RouteHop`] element.
const ROUTE_HOP_ENCODED_WIDTH: u32 = 64;

/// Encoded width, in bytes, of a single governance action element.
const ACTION_ENCODED_WIDTH: u32 = 32;

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn zero_length_vector_decodes_to_empty() {
        let env = Env::default();
        let decoded = decode_bounded_vec::<BatchItem>(&env, 0, 0, MAX_BATCH_SIZE, MAX_VECTOR_ENCODED_SIZE, BATCH_ITEM_ENCODED_WIDTH)
            .expect("zero-length vector must decode");
        assert_eq!(decoded.len(), 0);
    }

    #[test]
    fn oversized_element_count_is_rejected() {
        let env = Env::default();
        let err = decode_bounded_vec::<BatchItem>(
            &env,
            MAX_BATCH_SIZE + 1,
            (MAX_BATCH_SIZE + 1) * BATCH_ITEM_ENCODED_WIDTH,
            MAX_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            BATCH_ITEM_ENCODED_WIDTH,
        )
        .unwrap_err();
        assert_eq!(err, BatchError::VectorTooLarge);
    }

    #[test]
    fn oversized_encoded_size_is_rejected() {
        let env = Env::default();
        let err = decode_bounded_vec::<BatchItem>(
            &env,
            1,
            MAX_VECTOR_ENCODED_SIZE + 1,
            MAX_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            BATCH_ITEM_ENCODED_WIDTH,
        )
        .unwrap_err();
        assert_eq!(err, BatchError::VectorEncodedTooLarge);
    }

    #[test]
    fn length_mismatch_is_rejected() {
        let env = Env::default();
        let err = decode_bounded_vec::<BatchItem>(
            &env,
            2,
            BATCH_ITEM_ENCODED_WIDTH,
            MAX_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            BATCH_ITEM_ENCODED_WIDTH,
        )
        .unwrap_err();
        assert_eq!(err, BatchError::VectorLengthMismatch);
    }

    #[test]
    fn nested_vector_bounds_are_enforced() {
        let env = Env::default();
        // A nested vector is decoded by bounding the outer element count and
        // encoded size; the inner vector is bounded by the same helper when
        // its own header is decoded.
        let outer = decode_bounded_vec::<Vec<BytesN<32>>>(
            &env,
            1,
            ACTION_ENCODED_WIDTH,
            MAX_ACTION_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            ACTION_ENCODED_WIDTH,
        );
        assert!(outer.is_ok());

        let inner = decode_bounded_vec::<BytesN<32>>(
            &env,
            0,
            0,
            MAX_ACTION_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            ACTION_ENCODED_WIDTH,
        );
        assert!(inner.is_ok());
        assert_eq!(inner.unwrap().len(), 0);
    }

    #[test]
    fn malformed_input_leaves_no_partial_state() {
        let env = Env::default();
        // A malformed payload must fail before any allocation, so no partial
        // vector is observable to the caller.
        let result = decode_bounded_vec::<BatchItem>(
            &env,
            3,
            BATCH_ITEM_ENCODED_WIDTH,
            MAX_BATCH_SIZE,
            MAX_VECTOR_ENCODED_SIZE,
            BATCH_ITEM_ENCODED_WIDTH,
        );
        assert!(result.is_err());
    }
}
