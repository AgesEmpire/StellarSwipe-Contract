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
//! - Trade routes are validated for asset continuity before execution, so an
//!   invalid route fails without performing transfers or state changes.

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
    /// A trade route hop did not consume the prior hop's output asset.
    AssetContinuityBroken = 6,
    /// A trade route did not terminate in the requested asset.
    RouteTerminalAssetMismatch = 7,
    /// The trade route was empty.
    EmptyRoute = 8,
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

    /// Validate asset continuity across a trade route before execution begins.
    ///
    /// Each hop must consume the output asset of the prior hop, and the route
    /// must terminate in `requested_asset`. An empty route is rejected with
    /// [`BatchError::EmptyRoute`]. A hop whose `asset_in` does not match the
    /// prior hop's `asset_out` is rejected with
    /// [`BatchError::AssetContinuityBroken`]. A route whose final `asset_out`
    /// does not equal `requested_asset` is rejected with
    /// [`BatchError::RouteTerminalAssetMismatch`].
    ///
    /// This performs no transfers and mutates no state, so an invalid route
    /// fails before any irreversible effects occur.
    pub fn validate_route(
        route: &Vec<RouteHop>,
        requested_asset: &BytesN<32>,
    ) -> Result<(), BatchError> {
        let len = route.len();
        if len == 0 {
            return Err(BatchError::EmptyRoute);
        }

        let mut prev_out = route.get_unchecked(0).asset_in.clone();
        for i in 0..len {
            let hop = route.get_unchecked(i);
            if hop.asset_in != prev_out {
                return Err(BatchError::AssetContinuityBroken);
            }
            prev_out = hop.asset_out.clone();
        }

        if prev_out != *requested_asset {
            return Err(BatchError::RouteTerminalAssetMismatch);
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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn asset(env: &Env, seed: u8) -> BytesN<32> {
        BytesN::from_array(env, &[seed; 32])
    }

    fn hop(env: &Env, asset_in: u8, asset_out: u8) -> RouteHop {
        RouteHop {
            asset_in: asset(env, asset_in),
            asset_out: asset(env, asset_out),
        }
    }

    #[test]
    fn valid_chain_passes() {
        let env = Env::default();
        let mut route: Vec<RouteHop> = Vec::new(&env);
        route.push_back(hop(&env, 1, 2));
        route.push_back(hop(&env, 2, 3));
        route.push_back(hop(&env, 3, 4));
        assert_eq!(BatchVerifier::validate_route(&route, &asset(&env, 4)), Ok(()));
    }

    #[test]
    fn single_hop_chain_passes() {
        let env = Env::default();
        let mut route: Vec<RouteHop> = Vec::new();
        route.push_back(hop(&env, 1, 2));
        assert_eq!(BatchVerifier::validate_route(&route, &asset(&env, 2)), Ok(()));
    }

    #[test]
    fn disconnected_hops_fail() {
        let env = Env::default();
        let mut route: Vec<RouteHop> = Vec::new();
        route.push_back(hop(&env, 1, 2));
        route.push_back(hop(&env, 3, 4));
        assert_eq!(
            BatchVerifier::validate_route(&route, &asset(&env, 4)),
            Err(BatchError::AssetContinuityBroken)
        );
    }

    #[test]
    fn incorrect_final_asset_fails() {
        let env = Env::default();
        let mut route: Vec<RouteHop> = Vec::new();
        route.push_back(hop(&env, 1, 2));
        route.push_back(hop(&env, 2, 3));
        assert_eq!(
            BatchVerifier::validate_route(&route, &asset(&env, 9)),
            Err(BatchError::RouteTerminalAssetMismatch)
        );
    }

    #[test]
    fn empty_route_fails() {
        let env = Env::default();
        let route: Vec<RouteHop> = Vec::new(&env);
        assert_eq!(
            BatchVerifier::validate_route(&route, &asset(&env, 1)),
            Err(BatchError::EmptyRoute)
        );
    }
}
