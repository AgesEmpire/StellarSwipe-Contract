# Issue #253 Blocker

Issue #253 requests an oracle whitelist in `contracts/trade_executor/src/oracle.rs`.

This branch cannot safely implement that change on the current `main` base because:

- `contracts/trade_executor/src/oracle.rs` does not exist.
- `contracts/trade_executor/src/lib.rs` contains unresolved branch-marker text and duplicate/incomplete functions, so the contract does not parse.
- `contracts/trade_executor/src/errors.rs` also contains unresolved branch-marker text and conflicting error-code mappings.
- Workspace formatting is blocked by additional unrelated parse failures in `auto_trade` and `bridge`.

Required prerequisite:

1. Repair `trade_executor` so it has a valid module layout and compiles.
2. Decide whether #253 belongs in `trade_executor` or in the existing `auto_trade` oracle module, which already has whitelist-style code.
3. Reapply #253 against the repaired contract with tests for whitelisted update, unauthorized update, add oracle, remove oracle, and cannot remove last oracle.

This PR intentionally does not include `closes #253` because it does not implement the requested behavior.
