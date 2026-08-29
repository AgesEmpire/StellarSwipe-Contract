## Summary

<!-- What does this PR change, and why? -->

## Test plan

<!-- How was this verified? e.g. cargo test output, testnet deploy, new test cases added -->

## Security review checklist

Required for any change touching `contracts/` (or shared crates consumed by
contracts, e.g. `contracts/common`). Not required for docs-only, CI-only, or
non-contract tooling changes — if this PR doesn't touch contract code, check
the box below and move on.

- [ ] This PR does not touch `contracts/` or contract-consumed shared crates,
      **or** it does, and
      [`docs/security/release_security_checklist.md`](../docs/security/release_security_checklist.md)
      was reviewed against this change (Logic, Access control, Upgrades,
      Arithmetic categories as applicable).

The automated portion of the release gate runs in
[`security-release-gate.yml`](workflows/security-release-gate.yml)
(fmt/clippy/tests/deployment-manifest/error-code checks). This checkbox
covers the human-judgement half that automation can't verify.

## WASM ABI diff

CI posts a **WASM ABI Export Diff Report** as a PR comment. Check it for:
- [ ] All export changes are intentional.
- [ ] Removed or modified exports have a documented migration path.
- [ ] Any `.breaking.txt` file is removed in the follow-up PR.

## Related issue

<!-- e.g. Closes #123 -->
