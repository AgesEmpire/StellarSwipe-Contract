# StellarSwipe Threat Model

**Methodology:** STRIDE  
**Version:** 2.0.0  
**Status:** Awaiting external security reviewer sign-off  
**Last updated:** 2026-04-26  
**Scope:** All five production contracts — `oracle`, `auto_trade`, `signal_registry`, `stake_vault`, `governance`  
**Related docs:** `security_model.md`, `flash_loan_analysis.md`, `front_running_analysis.md`, `privilege_escalation_analysis.md`, `reentrancy_analysis.md`

---

## Contracts in Scope

| Contract | Role |
|---|---|
| **oracle** | Aggregates prices from SDEX and external adapters; provides TWAP/median/consensus to other contracts |
| **auto_trade** | Executes user-configured automated trades; manages positions, risk, stop-loss, and multi-strategy execution |
| **signal_registry** | Stores and manages trading signals; handles provider reputation, staking, and signal lifecycle |
| **stake_vault** | Custodies staked SEP-41 tokens; enforces lock periods and reentrancy-safe withdrawals |
| **governance** | Token-weighted voting, treasury management, timelocked proposal execution, committee governance |

> **Note:** `fee_collector`, `user_portfolio`, `bridge`, `trade_executor`, and shared libraries are referenced where they interact with the five contracts above but are not primary scope.

---

## STRIDE Threat Categories

| Category | Description |
|---|---|
| **S** — Spoofing | Impersonating a legitimate identity or contract |
| **T** — Tampering | Unauthorized modification of data or contract state |
| **R** — Repudiation | Denying an action occurred; lack of audit trail |
| **I** — Information Disclosure | Exposing data that should be private or confidential |
| **D** — Denial of Service | Preventing legitimate use of the protocol |
| **E** — Elevation of Privilege | Gaining capabilities beyond what is authorized |

---

## Threat Table Format

Each threat entry uses:

- **ID** — unique reference
- **Contract(s)** — affected contract(s)
- **STRIDE** — category
- **Description** — what the attacker does and what they gain
- **Likelihood** — HIGH / MEDIUM / LOW
- **Impact** — HIGH / MEDIUM / LOW
- **Status** — MITIGATED / PARTIAL / OPEN
- **Mitigation / Remediation** — controls in place or required

---

## S — Spoofing

### S-01: Fake oracle price submission

| Field | Value |
|---|---|
| **Contract** | oracle |
| **STRIDE** | Spoofing |
| **Description** | An attacker registers as an oracle operator and submits fabricated prices to skew the consensus output, causing incorrect stop-loss triggers or trade execution at wrong prices. |
| **Likelihood** | MEDIUM |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | Oracle operators are whitelisted by admin (`register_oracle` requires admin auth). Prices are aggregated via weighted median across all registered sources with a 300 s staleness filter, so a single rogue source is outvoted. Reputation and slashing logic (`reputation.rs`, `slash_oracle`) penalizes outlier submissions (>20% deviation triggers a slash). |
| **Residual / Remediation** | A colluding majority of oracle operators can still corrupt the median. Remediation: enforce a minimum operator count (≥3) before enabling price-dependent functions; add on-chain deviation alerts; require multi-sig for oracle operator registration. |

---

### S-02: Admin key impersonation at initialization

| Field | Value |
|---|---|
| **Contract** | oracle, auto_trade, signal_registry, stake_vault, governance |
| **STRIDE** | Spoofing |
| **Description** | If a contract is deployed but not immediately initialized, an attacker calls `initialize` / `init_admin` first and claims the admin role. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | MITIGATED |
| **Mitigation** | All five contracts guard `initialize` / `init_admin` with an "already set" check that panics or returns `AlreadyInitialized` on re-call. Deployment scripts must call `initialize` atomically in the same transaction as deployment. |

---

### S-03: Governance proposal executor impersonation

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Spoofing |
| **Description** | An attacker crafts a proposal that, when executed, calls a target contract pretending to be the governance contract itself, bypassing `require_auth` checks. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | MITIGATED |
| **Mitigation** | `execute_proposal` in governance only writes to governance-internal storage keys (`GovernanceParameters`, `GovernanceFeatures`, `GovernanceUpgrades`, `Treasury`). It does not invoke arbitrary cross-contract calls with governance's identity. Soroban `require_auth` enforces caller identity at the host level. Confirmed by privilege escalation analysis: no proposal type can write `StorageKey::Admin`. |

---

### S-04: Signal provider identity spoofing

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Spoofing |
| **Description** | An attacker submits signals under a reputable provider's address by replaying or forging authorization. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | All signal submission functions call `provider.require_auth()` via Soroban's host-enforced auth. Replay protection is provided by `stellar_swipe_common::replay_protection` (nonce / tx-hash dedup). The `increment_adoption` function additionally validates the caller against the registered `TradeExecutor` address. |

---

### S-05: Staker identity spoofing on withdrawal

| Field | Value |
|---|---|
| **Contract** | stake_vault |
| **STRIDE** | Spoofing |
| **Description** | An attacker calls `withdraw_stake` with a victim's address to drain their staked tokens to themselves. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | MITIGATED |
| **Mitigation** | `withdraw_stake` calls `staker.require_auth()` before any state read or token transfer. Soroban host-level auth prevents any caller from authorizing on behalf of another address without their signature. The token transfer destination is always `staker` (the authorized address), not the caller. |

---

### S-06: Cross-chain signal import with forged proof

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Spoofing |
| **Description** | An attacker submits a `request_signal_import` with a forged `proof` bytes value, claiming ownership of a signal from another chain and importing it under their Stellar address. |
| **Likelihood** | MEDIUM |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | `import_verified_signal` calls `cross_chain::verify_proof` before creating the Stellar signal. Address mapping must be pre-registered via `register_cross_chain_address` with a proof. |
| **Residual / Remediation** | `verify_proof` is currently a placeholder (`unimplemented!` stub in `cross_chain.rs`). Remediation: implement cryptographic proof verification (e.g., ECDSA signature from the source chain) before enabling cross-chain import on mainnet. |

---

## T — Tampering

### T-01: Oracle price history manipulation via sustained submission

| Field | Value |
|---|---|
| **Contract** | oracle |
| **STRIDE** | Tampering |
| **Description** | An attacker with oracle operator access submits a sequence of prices that gradually manipulates the TWAP stored in `history.rs`, causing `calculate_twap` to return a biased value used for stop-loss decisions. |
| **Likelihood** | MEDIUM |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | TWAP is computed over a configurable window of historical samples. Staleness filter (300 s) limits how quickly history can be poisoned. Reputation slashing penalizes outlier submissions (>20% deviation). Weighted median reduces influence of low-reputation oracles. |
| **Residual / Remediation** | A patient attacker with sustained oracle access can gradually shift TWAP. Remediation: add a maximum per-submission deviation cap (reject submissions >10% from current median); increase minimum oracle operator count to ≥3; add a circuit breaker that pauses price-dependent functions when TWAP deviation exceeds a threshold. |

---

### T-02: Signal data tampering post-submission

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Tampering |
| **Description** | A signal provider edits a signal after it has been acted upon by copy traders, retroactively changing parameters to claim better performance metrics or mislead followers. |
| **Likelihood** | MEDIUM |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | `update_signal` enforces a 60-second edit window (`now - submitted_at > 60` → `EditWindowClosed`). Edits are blocked if `adoption_count > 0` (`SignalAlreadyCopied`). Signal versioning (`versioning.rs`) records each edit as a new version with a timestamp. |
| **Residual / Remediation** | The 60-second window is tight but the adoption-count guard is the stronger protection. Remediation: emit a `SignalEdited` event with old and new version hashes (already done via `emit_signal_edited`); ensure indexers attribute copy trade performance to the version active at execution time. |

---

### T-03: Fee parameter tampering

| Field | Value |
|---|---|
| **Contract** | auto_trade, signal_registry, governance |
| **STRIDE** | Tampering |
| **Description** | Admin (or a governance proposal) sets fee rates to zero or to an extreme value, extracting value from users or breaking the fee mechanism. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | Fee parameters are bounded by on-chain caps (`fee_rate_bps` validated against `MAX_FEE_BPS` in `fees.rs`). Governance parameter changes go through a timelock delay. Admin key is protected by two-step transfer with 48 h expiry. `set_trade_fee` in signal_registry requires admin auth. |

---

### T-04: Governance treasury drain via malicious proposal

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Tampering |
| **Description** | An attacker with sufficient voting power passes a `TreasurySpend` proposal to drain the treasury to an attacker-controlled address. |
| **Likelihood** | MEDIUM |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | Treasury spend proposals require quorum and approval threshold. Timelock delay (`timelock.rs`) gives the community time to react. Guardian can cancel queued actions. `execute_treasury_spend` requires admin auth for direct spends. `BudgetExceeded` error prevents single proposals from exceeding 10% of treasury. |
| **Residual / Remediation** | If an attacker accumulates >50% of voting power (governance capture), the timelock is the only remaining defense. Remediation: implement a per-proposal spending cap enforced in `execute_proposal`; require committee approval for treasury spends above a threshold; add a guardian veto for high-impact proposals. |

---

### T-05: Stake balance manipulation via migration key collision

| Field | Value |
|---|---|
| **Contract** | stake_vault |
| **STRIDE** | Tampering |
| **Description** | An attacker exploits a storage key collision between `MigrationKey::StakesV2` and another storage key to overwrite stake balances, either inflating their own stake or zeroing a victim's. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | MITIGATED |
| **Mitigation** | `MigrationKey` is a `contracttype` enum; Soroban serializes enum variants with their discriminant, making collisions with other key types structurally impossible. The `StakesV2` map is keyed by `Address`, so one staker cannot affect another's entry. Storage key analysis (`storage_key_analysis.md`) confirms no collisions across all contracts. |

---

### T-06: Signal adoption count double-increment

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Tampering |
| **Description** | The `TradeExecutor` contract (or a compromised caller) calls `increment_adoption` multiple times with the same `(signal_id, nonce)` pair, inflating a signal's adoption count to boost provider reputation fraudulently. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | `increment_adoption` checks the `AdoptionNonces` map for `(signal_id, nonce)` before incrementing. If the nonce is already recorded, it returns `InvalidParameter`. Only the registered `TradeExecutor` address can call this function. |

---

### T-07: Vesting schedule manipulation

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Tampering |
| **Description** | Admin creates a vesting schedule with a zero cliff and zero duration, allowing immediate full release of tokens to an attacker-controlled beneficiary, bypassing the intended vesting timeline. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | `create_vesting_schedule` requires admin auth. `release_vested_tokens` requires beneficiary auth. `releasable_amount` in `distribution.rs` computes the vested fraction based on elapsed time relative to cliff and duration. |
| **Residual / Remediation** | Admin can set cliff=0 and duration=0, making all tokens immediately releasable. Remediation: enforce minimum cliff and duration values in `create_vesting_schedule`; require governance proposal approval for vesting schedules above a threshold amount. |

---

## R — Repudiation

### R-01: Oracle price submission without audit trail

| Field | Value |
|---|---|
| **Contract** | oracle |
| **STRIDE** | Repudiation |
| **Description** | An oracle operator submits a manipulated price and later denies it, claiming the on-chain record is insufficient to attribute the submission. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | `events.rs` in oracle emits `PriceSubmitted` for every `submit_price` and `submit_pair_price` call, including the submitter's address, asset pair, price, and timestamp. `emit_oracle_slashed` records slash events with reason. Soroban events are immutable ledger records. Per-operator accuracy history is maintained in `reputation.rs`. |

---

### R-02: Trade execution without user consent record

| Field | Value |
|---|---|
| **Contract** | auto_trade |
| **STRIDE** | Repudiation |
| **Description** | A user claims they did not authorize a trade that was executed on their behalf (e.g., via automated strategy or copy trade). |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | `execute_trade` calls `user.require_auth()` before execution. A `trade_executed` event is emitted with user address, signal ID, amount, price, and timestamp. `history.rs` stores per-user trade history on-chain. `auth.rs` records explicit authorization grants with expiry. |

---

### R-03: Governance vote repudiation

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Repudiation |
| **Description** | A voter claims they did not cast a vote or that their vote was miscounted. |
| **Likelihood** | LOW |
| **Impact** | LOW |
| **Status** | MITIGATED |
| **Mitigation** | `voting.rs` stores each vote on-chain keyed by `(proposal_id, voter)` in `VoteRecords`. Vote events are emitted. `calculate_proposal_statistics` provides a verifiable tally. Quadratic and conviction voting variants also store per-voter state. `AlreadyVoted` error prevents double-voting. |

---

### R-04: Signal performance outcome dispute

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Repudiation |
| **Description** | A signal provider disputes the recorded performance outcome of their signal, claiming the on-chain record was set incorrectly or by an unauthorized caller. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | `record_signal_outcome` is restricted to the registered `TradeExecutor` address (`caller != executor → Unauthorized`). Outcomes are idempotent — `OutcomeAlreadyRecorded` prevents overwriting. `emit_reputation_updated` records old and new scores. The `RecordedSignalOutcomes` map provides an immutable audit trail. |

---

### R-05: Stake withdrawal without on-chain record

| Field | Value |
|---|---|
| **Contract** | stake_vault |
| **STRIDE** | Repudiation |
| **Description** | A staker withdraws their tokens and later claims the withdrawal never happened, attempting to re-withdraw or dispute their balance. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | `withdraw_stake` zeroes the balance in `StakesV2` before the token transfer (checks-effects-interactions). The balance change is persistent and verifiable on-chain. |
| **Residual / Remediation** | No withdrawal event is emitted from `stake_vault`. Remediation: add a `WithdrawStake` event emission in `do_withdraw` with staker address, amount, and timestamp to provide a complete audit trail. |

---

### R-06: Admin action without event trail

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Repudiation |
| **Description** | An admin performs sensitive operations (treasury spend, vesting creation, committee dissolution) and later disputes having done so. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | All admin functions in governance emit events via `emit_admin_action` with the action symbol, actor address, and value. Specific events are emitted for vesting (`emit_vesting_created`), staking (`emit_stake_changed`), and rewards (`emit_reward_claimed`). All events are immutable Soroban ledger records. |

---

## I — Information Disclosure

### I-01: Iceberg order size leakage

| Field | Value |
|---|---|
| **Contract** | auto_trade |
| **STRIDE** | Information Disclosure |
| **Description** | The full size of an iceberg order is visible on-chain, allowing front-runners to anticipate the remaining order volume and trade ahead of it. |
| **Likelihood** | MEDIUM |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | `iceberg.rs` exposes `get_public_order_view` (shows only the visible slice) vs `get_full_order_view` (requires owner auth). The public view intentionally hides total size. |
| **Residual / Remediation** | On-chain storage is always readable by node operators querying ledger state directly, bypassing the contract's access control. Remediation: document this as a known limitation; for high-value orders, consider off-chain order management with on-chain settlement only. Encrypting total size on-chain is impractical without ZK proofs. |

---

### I-02: User portfolio position disclosure

| Field | Value |
|---|---|
| **Contract** | auto_trade |
| **STRIDE** | Information Disclosure |
| **Description** | An attacker reads a user's open positions from contract storage to target them with front-running or coordinated liquidation attacks. |
| **Likelihood** | MEDIUM |
| **Impact** | LOW |
| **Status** | OPEN |
| **Description** | Soroban contract storage is publicly readable. All position data (entry price, size, stop-loss level) is visible to any ledger observer. This is an inherent property of public blockchains. |
| **Remediation** | Document as a known limitation in user-facing materials. For high-value users, consider off-chain position management with on-chain settlement only. No on-chain fix is practical without ZK proofs. |

---

### I-03: Signal strategy leakage via on-chain parameters

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Information Disclosure |
| **Description** | A signal's full parameters (entry price, target, stop-loss, category, timing) are visible on-chain before the signal expires, allowing competitors to copy the strategy without attribution. |
| **Likelihood** | HIGH |
| **Impact** | LOW |
| **Status** | OPEN |
| **Description** | Signals are intentionally public by design — transparency is the product's value proposition. This is a design choice, not a vulnerability. |
| **Remediation** | For providers who want private signals, implement a commit-reveal pattern: publish a hash on-chain, reveal parameters only to subscribers off-chain. The `PREMIUM` category with `check_subscription` gating is a partial mitigation for access control, but does not hide on-chain storage from node operators. |

---

### I-04: Oracle operator list disclosure

| Field | Value |
|---|---|
| **Contract** | oracle |
| **STRIDE** | Information Disclosure |
| **Description** | The list of oracle operators is publicly readable, allowing an attacker to target individual operators for social engineering or key compromise. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | OPEN |
| **Description** | Oracle operator addresses are stored in persistent storage (`StorageKey::Oracles`) and are readable by anyone. This is inherent to on-chain transparency. |
| **Remediation** | Operational: use hardware-backed keys for oracle operators; rotate keys regularly; use multisig for oracle submissions where possible. Consider using contract addresses (rather than EOAs) as oracle operators to add an extra auth layer. |

---

### I-05: Governance token holder list disclosure

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Information Disclosure |
| **Description** | The full list of token holders and their balances is readable from `StorageKey::Holders` and `StorageKey::Balances`, enabling targeted attacks on large holders or revealing strategic voting power distribution. |
| **Likelihood** | LOW |
| **Impact** | LOW |
| **Status** | OPEN |
| **Description** | Token holder data is inherently public on-chain. This is expected behavior for a governance token. |
| **Remediation** | Document as expected behavior. Governance participants should be aware their voting power is public. No on-chain fix is practical or desirable for a transparent governance system. |

---

### I-06: Stake lock expiry timing disclosure

| Field | Value |
|---|---|
| **Contract** | stake_vault |
| **STRIDE** | Information Disclosure |
| **Description** | An attacker reads `StakesV2` to learn when a large staker's lock expires, then front-runs the withdrawal to manipulate market conditions or governance voting power at the moment of unlock. |
| **Likelihood** | LOW |
| **Impact** | LOW |
| **Status** | OPEN |
| **Description** | Lock expiry timestamps are stored in persistent storage and are publicly readable. This is inherent to on-chain transparency. |
| **Remediation** | Document as a known limitation. For governance-sensitive stakes, consider a withdrawal delay (cooldown period) after lock expiry to reduce the predictability of large unlock events. |

---

## D — Denial of Service

### D-01: Oracle staleness attack (liveness DoS)

| Field | Value |
|---|---|
| **Contract** | oracle |
| **STRIDE** | Denial of Service |
| **Description** | All oracle operators go offline or are coerced to stop submitting prices. The staleness filter causes `get_price_with_confidence` to return `StalePrice`, blocking all price-dependent operations (stop-loss, trade execution). |
| **Likelihood** | MEDIUM |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | `staleness.rs` and `OracleHealth` track per-operator health. `health_check` exposes liveness status for monitoring. Emergency pause allows admin to halt dependent contracts gracefully. `check_oracle_heartbeat` emits `HeartbeatMissed` events for off-chain alerting. |
| **Residual / Remediation** | No automatic fallback price source when all operators are stale. Remediation: integrate a secondary price source (e.g., on-chain SDEX TWAP as fallback with a wider staleness window); set up off-chain monitoring alerts for oracle liveness; define a minimum operator count below which the oracle auto-pauses. |

---

### D-02: Storage bloat via signal spam

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Denial of Service |
| **Description** | An attacker submits thousands of signals to bloat contract storage, increasing ledger fees for all users and potentially hitting Soroban storage limits. |
| **Likelihood** | MEDIUM |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | Signal submission requires a fee (`fees.rs`). Rate limiting (`stellar_swipe_common::rate_limit`) restricts submissions per address per time window. Staking requirement (`stake.rs`) raises the cost of spam. Signal expiry (`expiry.rs`) cleans up old entries. `cleanup_expired_signals` and `archive_old_signals` provide batch garbage collection. |
| **Residual / Remediation** | Rate limits and fees may be insufficient if the attacker is well-funded. Remediation: increase minimum stake for signal submission; implement automatic garbage collection triggered by storage size thresholds. |

---

### D-03: Flash loan price manipulation causing false stop-loss triggers

| Field | Value |
|---|---|
| **Contract** | auto_trade |
| **STRIDE** | Denial of Service |
| **Description** | An attacker uses a flash loan to temporarily move the SDEX spot price within a single transaction, triggering stop-losses for many users and forcing them out of positions at unfavorable prices. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | MITIGATED |
| **Mitigation** | Stop-loss evaluation in `auto_trade` uses the oracle contract price (`oracle::get_oracle_price`) when configured, not SDEX spot. The oracle uses median aggregation with a 300 s staleness filter — a single-transaction flash loan cannot manipulate a time-weighted multi-source median. When no oracle is configured, `signal.price` (pre-validated, not live SDEX) is used as fallback. See `flash_loan_analysis.md` for full analysis. |

---

### D-04: Governance proposal spam

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Denial of Service |
| **Description** | An attacker creates many governance proposals to overwhelm the voting system, causing voter fatigue and preventing legitimate proposals from receiving attention. |
| **Likelihood** | MEDIUM |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | Proposal creation requires a minimum staked token balance (`min_proposal_threshold` in `GovernanceConfig`). Proposals have a defined lifecycle with expiry. `NoVotingPower` error blocks zero-stake proposers. |
| **Residual / Remediation** | Remediation: add a proposal deposit that is slashed for proposals that fail to reach quorum; limit concurrent active proposals per address; implement a cooldown period between proposals from the same address. |

---

### D-05: Reentrancy-based DoS on `withdraw_stake`

| Field | Value |
|---|---|
| **Contract** | stake_vault |
| **STRIDE** | Denial of Service |
| **Description** | A malicious SEP-41 token contract re-enters `withdraw_stake` during the token transfer callback, attempting to drain the vault or leave the reentrancy lock permanently set. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | MITIGATED |
| **Mitigation** | `withdraw_stake` uses a temporary-storage reentrancy lock (`"WithdrawLock"`). The balance is zeroed in `StakesV2` before the token transfer (checks-effects-interactions). The lock is cleared on both success and error paths via `env.storage().temporary().remove`. A reentrant call finds the lock set and returns `ReentrancyDetected`. See `reentrancy_analysis.md` for full analysis. |

---

### D-06: Rate limit exhaustion via automated signal submission

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Denial of Service |
| **Description** | An attacker with a high-trust score (which raises rate limits) submits signals at the maximum allowed rate to exhaust the rate limit window for legitimate providers sharing the same window parameters. |
| **Likelihood** | LOW |
| **Impact** | LOW |
| **Status** | MITIGATED |
| **Mitigation** | Rate limits in `stellar_swipe_common::rate_limit` are per-address, not global. Each provider has an independent window. Trust score increases an individual provider's limit, not a shared pool. |

---

### D-07: Governance timelock queue exhaustion

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Denial of Service |
| **Description** | An attacker with sufficient voting power passes many proposals and queues all of them in the timelock simultaneously, exhausting the queue capacity and blocking legitimate actions. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | Each queued action has a unique `action_id`. The guardian can cancel queued actions. Proposal creation requires minimum voting power. |
| **Residual / Remediation** | No explicit queue size limit is enforced in `timelock.rs`. Remediation: add a maximum concurrent queued actions limit; require a higher voting threshold for proposals that queue timelock actions. |

---

### D-08: Oracle consensus round stall via submission withholding

| Field | Value |
|---|---|
| **Contract** | oracle |
| **STRIDE** | Denial of Service |
| **Description** | Oracle operators withhold submissions, preventing `calculate_consensus` from having enough data to produce a valid consensus price, stalling all downstream price-dependent operations. |
| **Likelihood** | MEDIUM |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | `calculate_consensus` returns `InsufficientOracles` if the submissions list is empty. The circuit breaker in `auto_trade` (`check_oracle_circuit_breaker`) detects oracle unavailability and blocks trading, preventing execution at stale prices. Admin can override the circuit breaker for emergency recovery. |
| **Residual / Remediation** | Remediation: define a minimum submission count threshold for consensus; implement automatic fallback to SDEX TWAP when consensus cannot be reached; set up off-chain monitoring for submission liveness. |

---

## E — Elevation of Privilege

### E-01: Admin key compromise → full protocol control

| Field | Value |
|---|---|
| **Contract** | oracle, auto_trade, signal_registry, stake_vault, governance |
| **STRIDE** | Elevation of Privilege |
| **Description** | An attacker compromises the admin private key and gains full control: can change fee rates, register malicious oracles, pause/unpause contracts, drain treasury, and transfer admin to themselves permanently. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | Admin transfer requires a two-step flow (propose → accept) with a 48 h expiry window in `oracle`, `auto_trade`, and `signal_registry`. Multisig support in `signal_registry` raises the bar. Guardian role limits blast radius during incident response. |
| **Residual / Remediation** | `governance` and `stake_vault` have no admin rotation mechanism — a compromised key is permanent. Remediation: add two-step admin transfer to `governance` and `stake_vault`; require hardware-backed multisig for all admin keys before mainnet. |

---

### E-02: Governance capture via token concentration

| Field | Value |
|---|---|
| **Contract** | governance |
| **STRIDE** | Elevation of Privilege |
| **Description** | An attacker accumulates >50% of governance tokens (or delegates) and passes arbitrary proposals, including treasury drains, parameter changes, and contract upgrade hashes. |
| **Likelihood** | MEDIUM |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | Timelock delay gives the community time to react and the guardian to cancel malicious proposals. Quorum and approval thresholds prevent low-participation attacks. Conviction voting and quadratic voting reduce whale dominance. Committee approval is required for certain high-impact actions. |
| **Residual / Remediation** | Timelock is the last line of defense against a >50% attacker. Remediation: implement a guardian veto for high-impact proposals; add a maximum single-address voting power cap; require committee approval for treasury spends above a threshold. |

---

### E-03: Guardian role abuse

| Field | Value |
|---|---|
| **Contract** | auto_trade, signal_registry, oracle, governance |
| **STRIDE** | Elevation of Privilege |
| **Description** | A compromised guardian cancels all queued governance actions (including legitimate upgrades) and triggers emergency pauses, effectively holding the protocol hostage. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | PARTIAL |
| **Mitigation** | Guardian can only pause and cancel queued actions — it cannot execute proposals, change admin, or drain treasury. Guardian is set and revoked exclusively by admin (`require_admin` guard on `set_guardian` and `revoke_guardian`). Confirmed by privilege escalation analysis. |
| **Residual / Remediation** | A malicious guardian can indefinitely block governance execution by canceling every queued action. Remediation: add a time-bounded guardian role (auto-expiry after N days); require admin + guardian multisig for cancellation of high-priority actions. |

---

### E-04: Signal provider → privileged execution via malicious signal parameters

| Field | Value |
|---|---|
| **Contract** | signal_registry, auto_trade |
| **STRIDE** | Elevation of Privilege |
| **Description** | A signal provider crafts a signal with extreme parameters (e.g., `price = 0`, `amount = MAX_I128`) that, when auto-executed by a copy trader, causes an integer overflow or bypasses risk checks. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | `create_signal` panics if `price <= 0` is not validated (implicit via `InvalidPrice` in oracle). `risk.rs` in auto_trade validates trade parameters before execution. `validate_trade` checks amount bounds and price sanity. `checked_sub` / `checked_mul` are used in fee and amount calculations to prevent overflow. `execute_trade` returns `InvalidAmount` for `amount <= 0`. |
| **Residual / Remediation** | Remediation: add explicit `price > 0` validation in `create_signal_internal`; fuzz test signal parameter boundaries; add a maximum signal price cap relative to current oracle price. |

---

### E-05: Cross-contract privilege escalation via dependency address manipulation

| Field | Value |
|---|---|
| **Contract** | auto_trade |
| **STRIDE** | Elevation of Privilege |
| **Description** | Admin sets the oracle or portfolio contract address to a malicious contract that returns attacker-controlled values, causing `auto_trade` to execute trades based on fabricated data. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | Dependency addresses (oracle, SDEX router) are set by admin only (`set_oracle_address` requires admin auth). Admin is protected by two-step transfer. Oracle whitelist per asset pair (`add_oracle` / `remove_oracle`) provides additional granularity. |
| **Residual / Remediation** | Remediation: emit events when dependency addresses are changed (already done via `OracleAdded`/`OracleRemoved` events); add a timelock delay for dependency address updates; consider a governance vote for changing critical dependency addresses. |

---

### E-06: Stake vault admin → unauthorized token drain

| Field | Value |
|---|---|
| **Contract** | stake_vault |
| **STRIDE** | Elevation of Privilege |
| **Description** | The `stake_vault` admin (set at initialization) has no privileged functions beyond initialization, but a future upgrade could add admin-only functions that allow draining staked tokens. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | Current `stake_vault` implementation has no admin-privileged functions beyond `initialize`. The only token transfer is in `withdraw_stake`, which requires staker auth and only transfers to the staker. No admin drain path exists in the current code. |
| **Residual / Remediation** | No admin rotation mechanism exists. Remediation: add two-step admin transfer; document that any future admin-privileged functions must go through governance approval; add a timelock for any future admin-controlled token movements. |

---

### E-07: Oracle operator → governance parameter manipulation via price-triggered logic

| Field | Value |
|---|---|
| **Contract** | oracle, governance |
| **STRIDE** | Elevation of Privilege |
| **Description** | A compromised oracle operator submits prices that trigger automated governance actions (e.g., a price-triggered parameter change), escalating from oracle operator to governance actor. |
| **Likelihood** | LOW |
| **Impact** | MEDIUM |
| **Status** | MITIGATED |
| **Mitigation** | Governance proposals are not triggered by oracle prices. Proposal creation, voting, and execution are entirely separate from oracle price feeds. No cross-contract call from oracle to governance exists in the current codebase. Confirmed by privilege escalation analysis. |

---

### E-08: Multisig signer collusion in signal_registry

| Field | Value |
|---|---|
| **Contract** | signal_registry |
| **STRIDE** | Elevation of Privilege |
| **Description** | A threshold number of multisig signers collude to perform admin actions (pause trading, change fees, register malicious oracles) without the original admin's knowledge. |
| **Likelihood** | LOW |
| **Impact** | HIGH |
| **Status** | PARTIAL |
| **Mitigation** | Multisig is opt-in and enabled by the admin. Adding/removing signers requires existing admin auth. The threshold is configurable. All admin actions emit events. |
| **Residual / Remediation** | Once multisig is enabled, the threshold of signers can act as admin. Remediation: require a time-delay for multisig-approved actions; emit events for all multisig approvals; consider requiring the original admin key as one of the required signers. |

---

---

## Attack Vector Summary

The following specific attack vectors from the issue scope are addressed:

| Attack Vector | Relevant Threats | Status |
|---|---|---|
| Oracle manipulation | S-01, T-01, D-01, D-08 | PARTIAL |
| Governance attacks | T-04, T-07, E-02, E-03 | PARTIAL |
| Flash loans | D-03 | MITIGATED |
| Front-running | I-01, I-03 | PARTIAL / OPEN |
| Reentrancy | D-05 | MITIGATED |
| Privilege escalation | E-01, E-02, E-03, E-04, E-05, E-06, E-07, E-08 | PARTIAL |

---

## STRIDE Coverage Matrix

The following table confirms all six STRIDE categories are covered for all five contracts:

| Contract | S | T | R | I | D | E |
|---|---|---|---|---|---|---|
| **oracle** | S-01, S-02 | T-01 | R-01 | I-04 | D-01, D-08 | E-01, E-07 |
| **auto_trade** | S-02 | T-03 | R-02 | I-01, I-02 | D-03, D-06 | E-01, E-04, E-05 |
| **signal_registry** | S-02, S-04, S-06 | T-02, T-03, T-06 | R-04 | I-03 | D-02, D-06 | E-01, E-04, E-08 |
| **stake_vault** | S-02, S-05 | T-05 | R-05 | I-06 | D-05 | E-01, E-06 |
| **governance** | S-02, S-03 | T-04, T-07 | R-03, R-06 | I-05 | D-04, D-07 | E-01, E-02, E-03 |

---

## Open Threats — Remediation Plan

The following threats have status **OPEN** and require action before mainnet:

| ID | Threat | Contract | Priority | Remediation |
|---|---|---|---|---|
| I-02 | User portfolio position disclosure | auto_trade | LOW | Document as known limitation; consider off-chain position management for high-value users |
| I-03 | Signal strategy leakage | signal_registry | LOW | Document as design choice; offer optional commit-reveal for premium providers |
| I-04 | Oracle operator list disclosure | oracle | MEDIUM | Operational: hardware keys, key rotation, multisig for oracle submissions |
| I-05 | Governance token holder list disclosure | governance | LOW | Document as expected behavior for a transparent governance token |
| I-06 | Stake lock expiry timing disclosure | stake_vault | LOW | Document as known limitation; consider withdrawal cooldown period |

The following threats have status **PARTIAL** and have tracked remediation items:

| ID | Threat | Contract | Priority | Remediation |
|---|---|---|---|---|
| S-01 | Fake oracle price submission | oracle | HIGH | Enforce minimum operator count (≥3); add per-submission deviation cap |
| S-06 | Cross-chain signal import with forged proof | signal_registry | HIGH | Implement cryptographic proof verification before enabling cross-chain import on mainnet |
| T-01 | Oracle price history manipulation | oracle | HIGH | Add maximum per-submission deviation cap (>10% from median = reject) |
| T-04 | Governance treasury drain | governance | HIGH | Implement per-proposal spending cap; require committee approval for large spends |
| T-07 | Vesting schedule manipulation | governance | MEDIUM | Enforce minimum cliff/duration; require governance approval for large vesting schedules |
| E-01 | Admin key compromise | all | HIGH | Add two-step admin transfer to `governance` and `stake_vault`; require hardware multisig |
| E-02 | Governance capture | governance | HIGH | Add guardian veto for high-impact proposals; implement voting power cap |
| E-03 | Guardian role abuse | multiple | MEDIUM | Add time-bounded guardian role with auto-expiry |
| E-04 | Malicious signal parameters | signal_registry, auto_trade | MEDIUM | Add explicit `price > 0` validation in `create_signal_internal`; fuzz test boundaries |
| E-05 | Dependency address manipulation | auto_trade | MEDIUM | Add timelock for dependency address updates; emit change events |
| E-06 | Stake vault admin drain risk | stake_vault | MEDIUM | Add two-step admin transfer; document governance approval requirement for future admin functions |
| E-08 | Multisig signer collusion | signal_registry | MEDIUM | Add time-delay for multisig-approved actions; require original admin as required signer |
| D-01 | Oracle staleness DoS | oracle | HIGH | Integrate secondary price source fallback; set up liveness monitoring |
| D-04 | Governance proposal spam | governance | MEDIUM | Add proposal deposit with slashing for failed proposals; add per-address proposal cooldown |
| D-07 | Timelock queue exhaustion | governance | MEDIUM | Add maximum concurrent queued actions limit |
| D-08 | Oracle consensus round stall | oracle | MEDIUM | Define minimum submission count; implement SDEX TWAP fallback |
| R-05 | Stake withdrawal without event | stake_vault | LOW | Add `WithdrawStake` event emission in `do_withdraw` |
| T-02 | Signal data tampering | signal_registry | MEDIUM | Ensure indexers attribute performance to version active at execution time |

---

## Review Sign-off

This document must be reviewed by at least one external security contributor before the protocol is considered production-ready.

| Reviewer | Organization / Handle | Date | Sign-off |
|---|---|---|---|
| TBD | External Security Contributor | — | Pending |

To sign off, open a PR that adds your name and a comment confirming you have reviewed all threats and their mitigation status.
