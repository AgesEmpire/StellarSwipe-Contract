# Error Handling and Recovery Patterns

This document describes the standardized error handling strategy used across Stellar Swipe contracts.

## Error categories

- `Validation`: Input validation, malformed requests, invalid amounts.
- `Authorization`: Missing or incorrect auth, admin-only operations.
- `ExternalDependency`: Oracle failures, cross-contract invocation failures.
- `Arithmetic`: Overflow / division by zero / invalid math.
- `Upgrade`: Upgrade and migration state issues.
- `Network`: Real-time network condition or congestion pricing issues.
- `Recovery`: Errors that require retry or manual intervention.

## Recovery mechanisms

- Contract-level error reporting stores the latest error event and recovery recommendation.
- Failed fee collection operations can be queued for retry via an on-chain recovery queue.
- Automatic retries are exposed through dedicated retry helper methods.
- Error reports include a recovery strategy and timestamp for auditability.

## Cross-contract error normalization

All cross-contract call sites must convert downstream results into the local
contract error ABI through a single shared normalization path rather than
ad-hoc `match`/`unwrap` mapping. This keeps error semantics consistent and
preserves the context needed to debug failures that originate in another
contract.

### Normalization contract

A cross-contract call returns a `Result<T, E>` where `E` is the downstream
contract's error type. The normalization helper converts that result into the
local error ABI as follows:

1. **Success** — the `Ok` value is returned unchanged; no error is recorded.
2. **Known downstream failure** — a recognized downstream error code is mapped
to the corresponding local error variant, preserving context:
   - the originating contract address / identifier,
   - the operation or entrypoint that was invoked,
   - the downstream error code.
3. **Unknown / unrecognized downstream failure** — any error code that is not
   part of the recognized set is mapped to the documented fallback variant
   `ExternalDependency::UnknownDownstreamFailure` (see below).
4. **Malformed return** — a return value that cannot be decoded into the
   expected downstream error shape is treated as an unknown downstream failure
   and mapped to the same fallback variant, with the raw payload retained in
   the error context where possible.

### Fallback error variant

`ExternalDependency::UnknownDownstreamFailure` is the documented fallback for
any downstream failure that cannot be classified. It is intentionally part of
the `ExternalDependency` category so that off-chain monitoring can treat all
cross-contract failures uniformly while still surfacing the preserved context
fields (origin, operation, downstream code).

### Preserved context

Every normalized cross-contract error carries:

- `origin`: the contract that produced the failure.
- `operation`: the invoked entrypoint or logical operation.
- `code`: the downstream error code, or a sentinel when the return was
  malformed.

This context is what allows a caller to distinguish a genuine authorization
failure in a downstream contract from a validation failure or an unclassified
error, without re-parsing raw return data.

### Testing expectations

Integration tests for cross-contract boundaries must cover:

- **Success** — the downstream call succeeds and the value passes through
  unchanged.
- **Known failure** — a recognized downstream error is mapped to the correct
  local variant with context preserved.
- **Malformed return** — an undecodable return maps to
  `ExternalDependency::UnknownDownstreamFailure`.

## Documentation

Developers should use these categories when mapping contract errors to frontend alerts or off-chain monitoring systems.
