# StellarSwipe Incident Response Runbook

> **Purpose:** Pre-written response plans for the most likely production incidents.
> Improvising during an incident leads to mistakes and delays. Follow this runbook
> to reduce response time and minimise errors.
>
> **Last reviewed:** 2026-04-26  
> **Review cadence:** Every 90 days or after any incident.

---

## Contact List & Escalation Path

| Role | Name | Primary Contact | Secondary Contact |
|------|------|-----------------|-------------------|
| On-Call Engineer | Rotating | PagerDuty rotation | Slack `#oncall` |
| Tech Lead | TBD | Slack DM | Phone (in PagerDuty) |
| Security Lead | TBD | Slack DM | Phone (in PagerDuty) |
| Ops Lead | TBD | Slack DM | Phone (in PagerDuty) |
| Governance Lead | TBD | Slack DM | Phone (in PagerDuty) |
| Legal / Compliance | TBD | Email | Slack `#legal` |

**Escalation path:**

```
On-Call Engineer → Tech Lead → Security Lead → Ops Lead → Executive Team
```

- **P0 (Critical — funds at risk):** Page Tech Lead + Security Lead immediately.
- **P1 (High — degraded service):** Notify Tech Lead within 15 minutes.
- **P2 (Medium — monitoring anomaly):** Notify Tech Lead within 1 hour.

---

## Scenario 1: Oracle Failure

### Description
The price oracle contract stops returning valid prices, causing `get_pnl` to return
`unrealized_pnl: None` for all users with open positions. New positions may be opened
at stale or zero prices.

### Detection
- Monitoring alert: `oracle_price_staleness_seconds > 300`
- On-chain: `get_pnl` responses show `unrealized_pnl: None` across multiple users
- Frontend: "Price unavailable" banners appear for all asset pairs
- Log pattern: repeated `oracle unavailable` panics in contract execution traces

### Immediate Response (< 15 min)
1. **Page** On-Call Engineer and Tech Lead.
2. **Assess scope:** Query oracle contract directly — is it returning stale data or panicking?
3. **Pause new position opens** if oracle has been silent > 10 minutes:
   - Admin calls `set_oracle` with a known-good backup oracle address if one exists.
   - If no backup: coordinate with governance to pause trading via the governance contract.
4. **Notify users** via status page and Slack `#announcements`: "Price feeds temporarily unavailable. Existing positions are safe. New opens are paused."
5. **Do NOT close positions** on behalf of users during an oracle outage — P&L cannot be calculated accurately.

### Investigation
1. Check oracle contract logs for the last successful `get_price` call.
2. Identify whether the failure is in the oracle contract itself or the upstream data feed.
3. Check Stellar network status for ledger congestion or validator issues.
4. Review recent oracle contract upgrades or admin key changes.

### Resolution
1. If upstream feed is restored: verify oracle returns valid prices, remove trading pause.
2. If oracle contract is broken: deploy a patched oracle, call `set_oracle` with the new address.
3. If a backup oracle was used: validate its prices against external sources before re-enabling.
4. Confirm `get_pnl` returns `unrealized_pnl: Some(...)` for test accounts.
5. Lift trading pause and notify users.

### Post-Mortem
- Document root cause, timeline, and user impact.
- Add oracle health check to CI/CD pipeline.
- Evaluate adding a secondary oracle fallback in the contract.
- Review SLA with oracle data provider.

---

## Scenario 2: Contract Exploit

### Description
An attacker exploits a vulnerability in one or more StellarSwipe contracts (e.g.,
`user_portfolio`, `trade_executor`, `fee_collector`) to drain funds, manipulate
positions, or bypass access controls.

### Detection
- Monitoring alert: abnormal outflow from fee collector or stake vault (> 2× daily average)
- On-chain: unexpected `position_closed` or `trade_cancelled` events for accounts not initiated by users
- Monitoring alert: `close_position_keeper` called by an address that is not the registered TradeExecutor
- User reports: positions closed without user action, unexpected balance changes
- Log pattern: auth bypass attempts, repeated calls from a single address in rapid succession

### Immediate Response (< 10 min — P0)
1. **Page** Tech Lead and Security Lead immediately.
2. **Freeze admin actions:** Do not execute any admin transactions until the attack vector is understood — the attacker may be monitoring admin activity.
3. **Assess blast radius:** Identify which contracts are affected and estimate funds at risk.
4. **Pause affected contracts** if a governance pause mechanism exists:
   - Call governance contract pause function (requires multisig).
   - If no pause: coordinate with Stellar validators to temporarily block contract invocations (last resort, requires network-level coordination).
5. **Do NOT upgrade contracts** under active attack — an upgrade could be front-run.
6. **Preserve evidence:** Capture all relevant transaction hashes, ledger numbers, and attacker addresses before any state changes.
7. **Notify users** via status page: "We have detected unusual activity and have paused trading as a precaution. Funds are being assessed. We will provide an update within 30 minutes."

### Investigation
1. Replay the attack transactions in a local Soroban environment to reproduce the exploit.
2. Identify the vulnerable code path (auth bypass, integer overflow, reentrancy, etc.).
3. Determine total funds affected and attacker address(es).
4. Check whether the attacker has already exited funds to external addresses.
5. Review recent contract upgrades and admin key activity for signs of insider involvement.

### Resolution
1. Develop and audit a patch for the vulnerability.
2. Deploy patched contract via governance upgrade procedure (see `docs/upgrade_procedure.md`).
3. If funds were drained: engage legal counsel and consider on-chain recovery options.
4. Restore service incrementally — start with read-only queries, then enable writes.
5. Notify all affected users with a detailed incident report.

### Post-Mortem
- Full public post-mortem within 72 hours.
- Engage external security auditors to review the patched code.
- Review and tighten auth model across all contracts.
- Consider a bug bounty programme if not already in place.

---

## Scenario 3: Governance Attack

### Description
An attacker accumulates sufficient governance tokens or exploits a governance contract
vulnerability to pass a malicious proposal — e.g., replacing the admin key, draining
the treasury, or upgrading contracts with backdoored code.

### Detection
- Monitoring alert: governance proposal created with unusually short voting period
- Monitoring alert: single address holds > 33% of voting power (configurable threshold)
- On-chain: proposal to change `Admin`, `TradeExecutor`, or contract WASM hash
- Monitoring alert: large token transfers to a single address in the 24 hours before a vote
- Community reports: suspicious proposal in governance forum

### Immediate Response (< 30 min)
1. **Page** Governance Lead, Tech Lead, and Security Lead.
2. **Analyse the proposal:** Identify exactly what the proposal would change if passed.
3. **Mobilise legitimate voters:** Alert the community via Discord, Twitter/X, and governance forum to vote against the malicious proposal.
4. **Check quorum and timeline:** Determine how much time remains before the proposal can be executed.
5. **If proposal has already passed but not yet executed:**
   - Coordinate with multisig holders to veto execution if the governance contract supports it.
   - If no veto mechanism: prepare a counter-proposal to revert the change immediately after execution.
6. **Do NOT transfer admin keys or treasury funds** until the situation is resolved.

### Investigation
1. Trace the origin of the attacker's voting power (token purchases, flash loans, delegation).
2. Identify whether the governance contract itself has a vulnerability or if this is a social/economic attack.
3. Review all recent governance proposals for related suspicious activity.
4. Check for collusion between multiple addresses.

### Resolution
1. If the malicious proposal is defeated: document the attack vector and propose governance parameter changes (e.g., longer voting periods, higher quorum requirements).
2. If the malicious proposal passed and was executed:
   - Immediately execute a counter-proposal to revert the change.
   - Rotate any compromised admin keys.
   - Audit all state changes made under the malicious governance action.
3. Implement governance safeguards: time-locks on execution, multi-sig veto, snapshot-based voting.

### Post-Mortem
- Publish a full governance incident report.
- Review tokenomics for concentration risk.
- Implement a governance security council with veto power for critical proposals.
- Add monitoring for large token movements before governance votes.

---

## Scenario 4: Key Compromise

### Description
An admin private key, multisig signer key, or deployer key is compromised, giving an
attacker the ability to call privileged contract functions (`set_oracle`, `set_trade_executor`,
`set_kyc_status`, contract upgrades, etc.).

### Detection
- Monitoring alert: admin function called from an unexpected IP or at an unusual time
- On-chain: `set_oracle`, `set_trade_executor`, or `set_kyc_status` called without a corresponding internal change request
- Team member reports a lost or stolen device containing key material
- Suspicious activity in key management system (HSM audit log, hardware wallet history)
- Unexpected contract upgrade transaction submitted

### Immediate Response (< 15 min — P0)
1. **Page** Tech Lead, Security Lead, and Ops Lead immediately.
2. **Assume the key is compromised** — do not wait for confirmation.
3. **Rotate the compromised key immediately:**
   - If multisig: coordinate all remaining signers to execute a key rotation transaction, replacing the compromised signer.
   - If single admin key: use the governance contract to propose and fast-track an admin key rotation (requires quorum).
4. **Revoke all active sessions** associated with the compromised key in any off-chain systems (CI/CD, deployment scripts, monitoring).
5. **Audit recent admin actions:** Review all transactions signed by the compromised key in the last 30 days.
6. **Freeze any changes** made by the attacker using the compromised key (e.g., revert oracle address, revert trade executor address).
7. **Notify users** if any user-facing state was modified: "We have detected unauthorised admin activity and have rotated our admin keys. We are reviewing all recent changes."

### Investigation
1. Determine how the key was compromised (phishing, malware, physical theft, insider threat).
2. Identify all transactions signed by the compromised key after the estimated compromise time.
3. Assess whether the attacker used the key to modify contract state or extract funds.
4. Review key management procedures for systemic weaknesses.

### Resolution
1. Complete key rotation and verify new key is operational.
2. Revert any malicious state changes made with the compromised key.
3. Conduct a full audit of all privileged operations since the compromise.
4. Update key management procedures to prevent recurrence (hardware security modules, air-gapped signing, stricter access controls).
5. If funds were affected: engage legal counsel and notify affected users.

### Post-Mortem
- Document the compromise vector and timeline.
- Review and update key management policy.
- Implement key usage monitoring and anomaly detection.
- Consider moving to a fully on-chain multisig with no single points of failure.
- Schedule a security training session for all team members with key access.

---

## General Incident Checklist

Use this checklist for any incident regardless of type:

- [ ] Incident declared and severity assigned (P0/P1/P2)
- [ ] Incident commander assigned
- [ ] Relevant team members paged
- [ ] Status page updated
- [ ] Evidence preserved (tx hashes, ledger numbers, logs)
- [ ] Blast radius assessed
- [ ] Mitigation applied
- [ ] Users notified
- [ ] Service restored
- [ ] Post-mortem scheduled (within 48 hours for P0, 1 week for P1/P2)
- [ ] Post-mortem published
- [ ] Action items tracked to completion
