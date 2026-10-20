# Cross-contract interface versioning

Cross-contract callers should treat the callee interface as a versioned API.

## Convention

- Each client wrapper records the interface version it was generated against.
- The callee exposes an `interface_version()` entrypoint that returns its current contract interface version.
- Before a caller proceeds with a cross-contract call, it must verify the callee reports the expected version.
- If the version does not match, the caller returns `CrossContractError::InterfaceVersionMismatch` instead of continuing into a downstream failure.

## When adding a new cross-contract call

1. Pick a version number for the callee interface and expose it through `interface_version()`.
2. Create or update the caller-side client wrapper with the expected version constant.
3. Verify compatibility before invoking the callee method.
4. Add a regression test that bumps the callee version and proves the caller rejects it.

## Compatibility rule

For now, compatibility is an exact match on the interface version. If a callee changes its interface, the caller must be updated explicitly.
