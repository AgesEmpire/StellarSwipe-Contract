# StellarSwipe Soroban contract upgrade procedure

This document is the **authoritative checklist** for upgrading deployed StellarSwipe contracts. Contract upgrades are high-risk; follow every section in order unless an emergency subsection explicitly says otherwise.

**Related runbooks**

- [Emergency pause](emergency_pause.md) — pause and unpause patterns before/after upgrades.
- Network and admin parameters: `config/testnet.json`, `config/mainnet.json`, and `scripts/deploy.ts` (see [PR_NETWORK_CONFIG](PR_NETWORK_CONFIG.md)).

**In-repo contracts (Wasm targets)**

Upgrade each deployed instance that is in scope for the release: `signal_registry`, `auto_trade`, `oracle`, `bridge`, `governance`, `fee_collector`, `user_portfolio`, `trade_executor`. Treat `common` as a library only (no separate deploy).

---

## Roles and approvals

| Role | Responsibility |
|------|----------------|
| Release owner | Coordinates timeline, owns the runbook checklist, comms. |
| Wasm builder | Produces release-tagged, reproducible Wasm artifacts and hashes. |
| Security / audit | Signs off on audit scope and critical findings (pre-mainnet). |
| Governance | Proposal creation, vote, timelock queue/execute where applicable. |
| Operators | Network transactions (upload, upgrade, invoke), monitoring. |

**Governance note:** Successful execution of a `ContractUpgrade` proposal records the **32-byte** new Wasm hash in governance storage (`ProposalType::ContractUpgrade`). That is the on-chain **approved** hash; operators must still perform the actual ledger upgrade using the upgrade authority for each contract instance. Default timelock delay for `ContractUpgrade` actions in this codebase is **5 days** (`5 * 86_400` seconds) unless governance has reconfigured it — plan votes and execution windows accordingly.

---

## Scenario matrix (all upgrade paths)

Use the row that matches the change; still complete **Pre-upgrade** and **Post-upgrade** for every path.

| Scenario | When it applies | Extra steps |
|----------|-----------------|-------------|
| **A. Planned semver release** | Routine features, non-breaking storage | Full governance vote, testnet rehearsal, staged mainnet rollout. |
| **B. Storage / migration release** | New `__post_upgrade__` or data layout changes | Extended testnet soak; migration invoke in **Upgrade**; rollback plan assumes **re-upgrade** only (see Rollback). |
| **C. Hotfix / security** | Exploit or critical bug | Emergency pause first; expedited governance if policy allows; shorten public exposure window; same technical upgrade steps. |
| **D. Multi-contract release** | Dependent contracts (e.g. registry + executor) | Define **strict order** (dependencies first); single change ticket; one coordinated unpause after all instances verify. |
| **E. Governance-only metadata** | Proposal records approved hash but ledger not yet updated | Treat as incomplete upgrade — complete ledger upgrade or cancel superseded proposal per policy. |

---

## Pre-upgrade

Complete all items; record completion and owner initials in the change ticket.

### 1. Change control

- [ ] **Ticket** opened with scope: contract names, network(s), target release tag / commit.
- [ ] **Risk rating** recorded (low / medium / high / critical).
- [ ] **Freeze** on unrelated contract deploys to the same IDs during the window.

### 2. Build and Wasm integrity

- [ ] Build **optimized** Wasm from a **pinned** commit (same as release tag).
- [ ] Compute and record the **Soroban Wasm hash** (32 bytes) for each artifact; match governance proposal `ContractUpgrade` validation (`hash.len() == 32`).
- [ ] Store artifacts in **immutable** release storage (checksum file alongside Wasm).
- [ ] **Reproducible build** verified where policy requires (second machine or CI reproduces same hash).

### 3. Audit and review

- [ ] **Diff review** complete: storage keys, auth, admin entrypoints, external calls, token/oracle interfaces.
- [ ] **Security audit** or internal security review signed off for **mainnet** scope (proportionate to risk).
- [ ] **Dependency** review (crates, SDK version) documented.

### 4. Automated tests

- [ ] `cargo test` (and Soroban tests) green on the release commit for all touched crates.
- [ ] CI policy respected (e.g. testnet-only config in CI — see `PR_NETWORK_CONFIG`).

### 5. Testnet rehearsal (mandatory before mainnet)

Treat this as **procedure validation**; capture evidence in the ticket.

- [ ] Deploy or identify **testnet** contract IDs matching production topology.
- [ ] Run **Pre-upgrade** checklist on testnet (audit may be abbreviated but not skipped for storage migrations).
- [ ] Execute full **Upgrade** section on testnet (pause → upload → ledger upgrade → migration if any → verify → unpause).
- [ ] Execute **Post-upgrade** checklist on testnet.
- [ ] Optional: execute **Rollback** drill (re-upgrade to previous known-good Wasm) on testnet.
- [ ] **Sign-off** in ticket: “Testnet upgrade procedure executed on \<date\>.”

### 6. Governance vote (mainnet and policy-governed testnet)

- [ ] Proposal text lists **contract name**, **network**, **contract ID**, **new Wasm hash**, and **release tag**.
- [ ] Voting quorum and outcome recorded.
- [ ] **Timelock** respected for `ContractUpgrade` (default **5 days** after queue unless configured otherwise).
- [ ] On execution, confirm governance storage reflects the approved hash for each `contract_name`.

### 7. Operational readiness

- [ ] **Upgrade authority** keys available (hardware-backed where required); signers roster confirmed.
- [ ] **RPC / horizon** endpoints stable; fee strategy agreed.
- [ ] **On-call** roster for 24h post-upgrade (see Post-upgrade).
- [ ] Stakeholders notified of **maintenance window** (if user-visible).

---

## Upgrade (execution day)

Perform in order for **each** contract instance in the agreed dependency order (Scenario D).

### 1. Pause user-facing operations

- [ ] Pause **all** in-scope contracts per [emergency_pause.md](emergency_pause.md) (use **granular** `pause_category` where the contract supports it, or legacy `pause` / full pause as documented).
- [ ] Confirm **mutators** fail with expected pause errors; **read-only** getters still succeed where required for verification.

### 2. Upload new Wasm to the ledger

- [ ] Upload/install Wasm so the network has the **exact** bytes matching the approved hash.
- [ ] Record ledger response (transaction hash, wasm id / hash as returned by tooling).

### 3. Apply ledger-level code upgrade

- [ ] Submit the **UpdateContractCode** (or equivalent) operation for the target contract ID using the **upgrade authority**, pointing at the uploaded Wasm hash.
- [ ] Confirm transaction success and that the instance now reports the expected **contract version** / `health_check` where implemented (e.g. governance `health_check` exposes `CARGO_PKG_VERSION`).

### 4. Migration (if applicable)

- [ ] If the new Wasm exposes a **migration** / post-upgrade entrypoint, invoke it **once** with the documented arguments.
- [ ] Verify **storage invariants** immediately after migration (spot-check critical keys via read-only calls or indexer).

### 5. Verification before unpause

- [ ] **Smoke tests** on paused or safe paths: version strings, config getters, zero-risk calls as defined in the release notes.
- [ ] **Indexer / events** spot-check for unexpected errors in the upgrade transaction window.

### 6. Unpause

- [ ] Unpause in **reverse** order of pause if dependencies matter; otherwise parallelize per runbook.
- [ ] Confirm traffic resumes and **no** sustained pause-related errors in logs.

---

## Post-upgrade

### Immediate (first hour)

- [ ] Run **contract-specific** smoke matrix from the release (deposit/withdraw/signal paths as applicable to each contract).
- [ ] Compare **metrics** (error rate, latency, event volume) to pre-upgrade baseline.
- [ ] Confirm **oracle / external** integrations still within SLA.

### First 24 hours

- [ ] **Active monitoring** on-call; page thresholds tightened if policy allows.
- [ ] **Daily summary** posted to the ticket (incidents, anomalies, resolved false positives).

### Close-out (after 24h stable)

- [ ] Ticket updated: “Post-upgrade monitoring complete.”
- [ ] **Artifact retention**: Wasm + hash + tx hashes archived with the release.
- [ ] Runbook updates (this file) if lessons learned change best practice.

---

## Rollback

Soroban does not offer a one-click “undo” of code on a live contract ID. **Rollback = return to a known-good Wasm** (usually the previous release) via another **ledger upgrade**, plus operational stabilization.

### Preconditions

- [ ] **Prior Wasm** binary and **32-byte hash** still available (from last release archive).
- [ ] Contracts **paused** (same as upgrade) if user impact must stop immediately.

### Steps

1. [ ] **Pause** all affected contracts ([emergency_pause.md](emergency_pause.md)).
2. [ ] **Upload** the previous (or hotfix-repair) Wasm if not still installable from chain history.
3. [ ] **Upgrade** contract ID(s) to the **rollback** Wasm hash using upgrade authority.
4. [ ] If the failed upgrade ran a **forward migration** that is incompatible with old code, **stop** and escalate — you may need a **forward-fix** Wasm instead of a naive rollback. Document state in the ticket.
5. [ ] **Verify** read-only and critical mutators on rollback code.
6. [ ] **Unpause** only after sign-off.
7. [ ] **Governance**: follow policy to record superseding proposal or emergency actions as required.

### When not to rollback on-chain

- **State corruption** or **irreversible migration** — prefer **paused** system + **new forward Wasm** that repairs state.
- **Key compromise** — rotate authorities per incident plan before unpausing.

---

## PR and release gate (done criteria)

Use this list before merging release branches and before mainnet execution.

- [ ] This procedure’s **testnet** path executed and **linked** in the PR / ticket.
- [ ] **Rollback** section reviewed for the specific release (migration notes).
- [ ] **PR reviewed by at least two team members** (required).
- [ ] Emergency contacts and **pause** ownership confirmed for the upgrade window.

---

## Appendix — operator command stubs

Replace placeholders: `NETWORK`, `SOURCE`, `CONTRACT_ID`, `WASM_PATH`, admin `Address` args, and tooling flags match your installed **Stellar / Soroban CLI** version.

**Pause (example pattern — align with [emergency_pause.md](emergency_pause.md))**

```bash
soroban contract invoke \
  --source SOURCE \
  --network NETWORK \
  --wasm PATH_TO_MATCHING_WASM_OR_FETCH \
  --id CONTRACT_ID \
  FUNCTION \
  --arg ADMIN_OR_CALLER
```

**Upload / install Wasm (conceptual)**

```bash
# Upload or install per your CLI; record the returned hash and compare to governance-approved 32-byte hash.
soroban contract install --wasm WASM_PATH --source SOURCE --network NETWORK
```

**Post-upgrade verification**

```bash
soroban contract invoke \
  --source SOURCE \
  --network NETWORK \
  --id CONTRACT_ID \
  --wasm WASM_PATH \
  health_check
```

> **Note:** Exact CLI subcommands and flags change between releases; always prefer the **official Stellar/Soroban documentation** for your CLI version and verify against a testnet dry run first.

---

*Document version: 1.0 — maintain alongside release process.*
