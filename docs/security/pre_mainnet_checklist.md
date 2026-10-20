# Pre-Mainnet Security Checklist

**Document Status**: Ready for Mainnet Launch  
**Last Updated**: 2026-05-27  
**Protocol Version**: 23+ (Soroban)

---

## Executive Summary

This document serves as the final security gate before mainnet deployment. All items require explicit sign-off from named team members. Any open items must have documented risk acceptance signed by the Security Lead and Protocol Lead.

---

## 1. SECURITY ISSUES RESOLVED

### 1.1 Audit Findings
- [ ] **All critical audit findings addressed**
  - Jira Tickets: [Link to audit tickets]
  - Verification: Code review + automated tests pass
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

- [ ] **All high-severity findings remediated**
  - Jira Tickets: [Link to findings]
  - Verification: Fixes verified in code review + regression tests added
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

### 1.2 Known Vulnerabilities
- [ ] **Dependencies scanned for CVEs**
  - Tool: cargo-audit v0.18+
  - Last scan date: ___________________
  - Results: 0 known vulnerabilities in dependencies
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (DevOps Lead)
  - Date: ___________________

- [ ] **No hardcoded secrets or credentials in codebase**
  - Tool: GitGuardian / custom secret scanner
  - Verification: Full codebase scan completed
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

### 1.3 Integer Overflow / Underflow
- [ ] **All numeric operations checked**
  - Modules: storage.rs, portfolio.rs, risk.rs, multi_asset.rs
  - Protection: Rust's safe arithmetic by default; explicit checks for division by zero
  - Test coverage: Integer bounds tests exist for all trade operations
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Protocol Lead)
  - Date: ___________________

### 1.4 Reentrancy & State Management
- [ ] **Reentrancy analysis completed**
  - Finding: Soroban's invocation model prevents reentrancy; no external calls within state mutations
  - Verification: Code review of all contract invocations
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

---

## 2. THREAT MODEL COMPLETE

### 2.1 Threat Categories Documented
- [ ] **Supply chain threats**
  - Scenario: Compromised dependency
  - Mitigation: Pinned versions, vendor verification, regular audits
  - Residual Risk: LOW
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (DevOps Lead)
  - Date: ___________________

- [ ] **Economic threats (flash loans, slippage, oracle manipulation)**
  - Scenario: Attacker manipulates oracle price to trigger bad trades
  - Mitigation: TWAP oracle with multi-hop validation; slippage limits in execute_trade
  - Residual Risk: MEDIUM (see section 3 for oracle mitigation)
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Protocol Lead)
  - Date: ___________________

- [ ] **Operational threats (key compromise, front-running)**
  - Scenario: Admin key compromised; attacker pauses contract / drains fees
  - Mitigation: Multi-sig admin (3-of-5); rate limiting on admin operations; grace period for position closes
  - Residual Risk: LOW
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

- [ ] **Denial of Service (DoS)**
  - Scenario: Attacker fills storage to prevent new trades
  - Mitigation: Per-user storage bounds; rent mechanism in Soroban limits spam
  - Residual Risk: LOW
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Infrastructure Lead)
  - Date: ___________________

### 2.2 Threat Response Procedures
- [ ] **Playbook for each HIGH threat documented**
  - Location: docs/security/incident_response.md
  - Triggers: Defined alert thresholds for each scenario
  - Response steps: Pause contract, investigate, notify stakeholders, coordinate fix
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Incident Commander)
  - Date: ___________________

---

## 3. ACCESS CONTROL AUDITED

### 3.1 Role-Based Access Control (RBAC)
- [ ] **Admin role restrictions**
  - Permissions: pause_contract, set_risk_defaults, transfer_admin, update_fees
  - Implementation: require_admin() check on all privileged operations
  - Multi-sig: 3-of-5 threshold for mainnet
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

- [ ] **Operator role restrictions**
  - Permissions: set_rate_limited(), clear_rate_limited(), execute_emergency_close
  - Implementation: require_operator() check; separate from admin
  - Multi-sig: 2-of-3 for mainnet
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Operations Lead)
  - Date: ___________________

- [ ] **User-level authorization**
  - Functions: execute_trade, cancel_copy_trade, update_risk_config
  - Implementation: Address::require_auth() on all user-initiated operations
  - Test coverage: 5+ tests verify unauthorized access is rejected
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Protocol Lead)
  - Date: ___________________

### 3.2 Cross-Contract Authorization
- [ ] **Oracle contract calls authorized**
  - Flow: auto_trade → oracle (get_price) → SDEX contracts
  - Verification: Oracle signatures checked; staleness enforced (max 5 min)
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

- [ ] **SDEX swap authorization**
  - Flow: auto_trade requests swap on SDEX; SDEX validates user signature
  - Verification: Each SDEX call includes user auth; no delegated swaps without explicit approval
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Protocol Lead)
  - Date: ___________________

---

## 4. ORACLE MANIPULATION MITIGATED

### 4.1 Oracle Architecture
- [ ] **TWAP oracle configured with adequate window**
  - Window: 60 ledgers (5 minutes); 900-second lookback
  - Verification: price = sum(price × delta_time) / total_time over window
  - Multi-source: Prices from 3+ liquidity pools; outliers removed (±15%)
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Oracle Lead)
  - Date: ___________________

- [ ] **Multi-hop validation active**
  - Scenario: Price discrepancy > 10% across hops triggers warning
  - Implementation: convert_via_hops() checks each intermediate price
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Protocol Lead)
  - Date: ___________________

### 4.2 Trade Execution Safeguards
- [ ] **Slippage limits enforced**
  - Limit: 2% slippage max on all trades (configurable per strategy)
  - Implementation: compare(execution_price, oracle_price); revert if diff > limit
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Protocol Lead)
  - Date: ___________________

- [ ] **Execute trade accepts min_amount_out parameter**
  - User specifies: "I will accept at least X tokens at price Y"
  - Fallback: If market price < min_amount_out, trade is cancelled
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Protocol Lead)
  - Date: ___________________

### 4.3 Oracle Freshness & Staleness
- [ ] **Stale oracle detection**
  - Threshold: 5 minutes (300 seconds; ~300 ledgers)
  - Implementation: if now - last_price_update > 5 min → emit StalePriceWarning; allow trade only if force_stale_ok=true (Operator only)
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Oracle Lead)
  - Date: ___________________

- [ ] **Oracle fallback mechanism**
  - Primary: Soroban native oracle (TWAP)
  - Fallback: External adapter for manual price submission (Operator-controlled)
  - Transition: Automatic if primary stale > 10 min
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Oracle Lead)
  - Date: ___________________

---

## 5. UPGRADE PROCEDURE TESTED

### 5.1 Contract Upgrade Flow
- [ ] **WASM migration tested on testnet**
  - Scenario: Deploy v1.1 (new TWAP window) over v1.0
  - Verification: All persistent storage correctly migrated; no data loss
  - Rollback procedure: Documented and tested
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Infrastructure Lead)
  - Date: ___________________

- [ ] **Data migration scripts tested**
  - Script: scripts/migrate_contract.sh
  - Test env: Testnet (full data replica)
  - Verification: Pre-migration integrity check + post-migration verification
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (DevOps Lead)
  - Date: ___________________

### 5.2 Governance for Upgrades
- [ ] **Upgrade proposal process documented**
  - Trigger: 7-day voting period; 66% supermajority required
  - Multi-sig: Final approval by 3-of-5 admin signers
  - Notification: 24-hour notice before execution
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Governance Lead)
  - Date: ___________________

- [ ] **Upgrade communication plan**
  - Channels: Discord, email, website banner
  - Content: Change summary, user impact, rollback plan
  - Timing: Announced at T-72h, T-24h, T-1h, and upon completion
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Communications Lead)
  - Date: ___________________

---

## 6. INCIDENT RESPONSE READY

### 6.1 Incident Response Plan
- [ ] **Incident classification defined**
  - CRITICAL: Funds at risk, exploit active (pause & investigate immediately)
  - HIGH: Large-scale service degradation (pause if necessary; investigate within 1h)
  - MEDIUM: Minor issues, 1-2 users affected (investigate within 4h)
  - LOW: Informational, no impact
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Incident Commander)
  - Date: ___________________

- [ ] **Incident contact tree established**
  - Incident Commander: ___________________
  - Security Lead: ___________________
  - Protocol Lead: ___________________
  - DevOps Lead: ___________________
  - Legal/Communications: ___________________
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Incident Commander)
  - Date: ___________________

### 6.2 Emergency Procedures
- [ ] **Contract pause procedure tested**
  - Function: pause_contract(admin) with 1-hour grace period for position closes
  - Test: Can close positions during grace period; all other operations blocked
  - Verification: Grace period auto-expires correctly
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Operations Lead)
  - Date: ___________________

- [ ] **Emergency fund recovery procedure**
  - Scenario: Exploit found; need to freeze attacker's positions and recover funds
  - Procedure: Emergency close + fee sweep (documented in emergency_recovery.md)
  - Approval: 5-of-5 admin multi-sig required
  - Test: Dry-run on testnet with simulated attack
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Security Lead)
  - Date: ___________________

### 6.3 Post-Incident Response
- [ ] **Postmortem template prepared**
  - Components: Timeline, root cause, impact analysis, lessons learned, preventive actions
  - Location: docs/security/postmortem_template.md
  - Timing: Public postmortem within 7 days of incident
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Governance Lead)
  - Date: ___________________

- [ ] **Communication templates ready**
  - Templates: User notification, staking rewards adjustment, fund recovery status
  - Channels: Discord, email, in-app notification
  - Legal review: Completed
  - Status: ⬜ PENDING
  - Sign-off: _____________________ (Communications Lead)
  - Date: ___________________

---

## 7. RISK ACCEPTANCE

Any item with **RED** status above must either be moved to **GREEN** or have documented risk acceptance below.

### 7.1 Accepted Risks

**Risk 1: [Description]**
- Item: [Reference to section above]
- Justification: [Why we accept this risk]
- Monitoring: [How we monitor]
- Mitigation at scale: [Future plan to address]
- Approved by: _____________________ (Security Lead)
- Approved by: _____________________ (Protocol Lead)
- Date: ___________________

---

## 8. SIGN-OFF SUMMARY

### All Items Complete: YES ☐ / NO ☐

| Role | Name | Signature | Date |
|------|------|-----------|------|
| Security Lead | _____ | _____________________ | __________ |
| Protocol Lead | _____ | _____________________ | __________ |
| Operations Lead | _____ | _____________________ | __________ |
| Infrastructure Lead | _____ | _____________________ | __________ |
| DevOps Lead | _____ | _____________________ | __________ |

---

## Appendices

### A. Audit Report Link
[Link to most recent third-party audit]

### B. Threat Model Document
[Link to detailed threat model]

### C. Incident Response Playbook
[Link to incident_response.md]

### D. Emergency Recovery Procedures
[Link to emergency_recovery.md]

### E. Test Coverage Report
[Link to coverage report showing >90% coverage]

### F. Dependency Lock File
[Link to Cargo.lock with pinned versions]

---

**Approval Date**: ___________________  
**Approved by**: _____________________ (Executive Lead)

This checklist must be completed and approved before any mainnet deployment.
