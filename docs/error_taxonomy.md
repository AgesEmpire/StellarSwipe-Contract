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

## Integration test coverage

Cross-contract boundaries are covered by integration tests for:

- **Success**: the downstream call succeeds and the result is returned
  unchanged.
- **Known failure**: a recognized downstream failure is normalized into the
  matching local error variant with preserved context.
- **Malformed return**: an unrecognized or malformed downstream return is mapped
  to the documented fallback error variant.
