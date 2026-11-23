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

## Interface artifacts (CI)

`stellar-swipe/cross-contract-interfaces/interfaces.json` records the method signatures of every `#[contractclient]` trait in the workspace, plus the `#[contracttype]` / `#[contracterror]` definitions those signatures use. `stellar-swipe/scripts/check_cross_contract_interfaces.py` regenerates that view from source and diffs it against the committed file:

- Removed or changed signatures and serialized types fail the build. Restore the old signature, or bump the version and update every dependent caller.
- Added methods, types or clients also fail until the artifact is regenerated, so the committed file never drifts from source.
- Types whose name is defined more than once in a crate are listed under `unresolved` and are not diffed.

Regenerate after an intentional change with:

```bash
python3 stellar-swipe/scripts/check_cross_contract_interfaces.py --update
```
