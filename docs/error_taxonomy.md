# Error Taxonomy

This document defines the canonical error taxonomy used across the protocol's
contracts and, in particular, how errors that originate in a downstream
contract are normalized at cross-contract call boundaries.

## Local error ABI

Every contract exposes a single local error enum. Cross-contract call sites
must not surface raw downstream errors; instead they convert any downstream
failure into the local error ABI through one shared normalization path.

## Normalization at contract boundaries

All cross-contract call sites use a single normalization helper. The helper:

1. Accepts the downstream result together with the originating context
   (originating contract and operation name).
2. Preserves error context when converting a downstream failure into the local
   error ABI, so the originating contract, the operation, and the downstream
   error code remain available to callers and to off-chain tooling.
3. Maps any unknown or unrecognized downstream failure to the documented
   fallback error variant described below.

Ad-hoc, per-call-site error mapping is not permitted; it leads to inconsistent
ABIs and lost context.

## Known downstream failures

Known downstream failures are those the local contract explicitly understands
(authorization failures, validation failures, and recognized downstream error
codes). These are converted into the corresponding local error variant while
retaining the originating contract, operation, and downstream error code.

## Fallback error

Any downstream failure that is not recognized — including malformed or
unexpected return values — is mapped to the documented fallback error variant.
The fallback variant carries the preserved context (originating contract,
operation, and, when available, the downstream error code) so that the failure
remains diagnosable even when its exact cause is unknown.

## Unexpected panics at contract boundaries

A panic is not an ordinary validation error and must never be reported as one.
Ordinary validation failures continue to surface their existing, documented
error codes unchanged; the panic handling described here applies only to
unexpected panics that escape a contract boundary.

Unexpected panics are made predictable at public contract boundaries:

- **Stable public error**: a panic that escapes a public contract boundary is
  mapped to the documented fallback error variant, using the same normalization
  path as any other unrecognized downstream failure. Callers therefore observe
  a stable, documented error code rather than an opaque trap or an
  implementation-specific panic message.
- **Preserved context**: the mapped error retains the originating contract and
  operation, so the panic remains diagnosable off-chain even though its exact
  cause is unknown.
- **No masking of validation errors**: because panics are mapped only through
  the fallback path, expected validation and authorization error codes are
  unaffected and remain byte-for-byte identical to their documented values.

### State rollback semantics

Panic handling must not leave partially applied state behind. When a panic is
mapped to the fallback error at a contract boundary, the state changes made by
the failing operation are rolled back, so the contract's observable state is the
same as if the operation had never been invoked. Callers that receive the
fallback error can rely on this rollback guarantee.

## Integration test coverage

Cross-contract boundaries are covered by integration tests for:

- **Success**: the downstream call succeeds and the result is returned
  unchanged.
- **Known failure**: a recognized downstream failure is normalized into the
  matching local error variant with preserved context.
- **Malformed return**: an unrecognized or malformed downstream return is mapped
  to the documented fallback error variant.
- **Panic path**: an unexpected panic escaping a contract boundary is mapped to
  the documented fallback error variant with preserved context, and the expected
  error codes for ordinary validation failures remain unchanged.
- **Rollback on panic**: state changes made by a panicking operation are rolled
  back, leaving the contract's observable state unchanged.
