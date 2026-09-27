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

## Stellar token response compatibility

Supported Stellar token contracts (SEP-41 style fungible tokens) are invoked
through the same normalization path as any other cross-contract call. This
section documents the response shapes the contracts assume and the failure
behavior when a token deviates from them.

### Supported response shapes

For the token entrypoints used by Stellar Swipe (`transfer`, `transfer_from`,
`balance`, `approve`, `allowance`), the following return shapes are supported:

- **`transfer` / `transfer_from` / `approve`** — a successful call returns
  `Ok(())` (unit). Any non-unit success payload is treated as malformed.
- **`balance` / `allowance`** — a successful call returns `Ok(i128)` with a
  non-negative amount. Negative amounts are treated as malformed.
- **Errors** — a failed call returns `Err(token_error)` where `token_error` is
  the token's own error enum. Recognized variants are mapped to local variants;
  everything else falls through to the fallback below.

### Assumptions

- The token contract is already deployed and its address is trusted; Stellar
  Swipe does not verify token bytecode.
- Amounts are expressed in the token's smallest unit and fit in `i128`.
- The token does not silently succeed on a failed transfer (no false `Ok(())`).
- Fee-on-transfer / rebasing tokens are out of scope and rejected elsewhere;
  this section only covers the response shape of the supported set.

### Fail-closed behavior

Unexpected token responses must never be treated as success. The normalization
path fails closed with a stable, documented error:

- **Unknown token error code** → `ExternalDependency::UnknownDownstreamFailure`.
- **Malformed success payload** (wrong type, negative balance, undecodable
  return) → `ExternalDependency::UnknownDownstreamFailure`.
- **Missing return value** where one is required →
  `ExternalDependency::UnknownDownstreamFailure`.

In every case the preserved context (`origin` = token address,
`operation` = entrypoint, `code` = downstream code or malformed sentinel) is
attached so callers and monitoring can distinguish a genuine token error from a
malformed response.

### Token response test coverage

Compatibility tests for supported Stellar assets must cover:

- **Valid returns** — `transfer`/`transfer_from`/`approve` returning `Ok(())`
  and `balance`/`allowance` returning `Ok(i128)` pass through unchanged.
- **Contract errors** — a recognized token error maps to the correct local
  variant with context preserved; an unrecognized token error maps to
  `ExternalDependency::UnknownDownstreamFailure`.
- **Malformed response fixtures** — fixtures that return the wrong type, a
  negative balance, or an undecodable payload all map to
  `ExternalDependency::UnknownDownstreamFailure` and never surface as success.

## Documentation

Developers should use these categories when mapping contract errors to frontend alerts or off-chain monitoring systems.
