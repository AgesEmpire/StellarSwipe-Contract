# StellarSwipe Security Model

**Version:** 1.0.0  
**Status:** Draft for external review  
**Last updated:** 2026-04-23

## Purpose

This document describes StellarSwipe's trust model and security assumptions so auditors, security researchers, and advanced users can evaluate protocol risk clearly.

Threat model reference documents:

- `docs/attack_vectors.md` (primary threat model and attack catalog)
- `docs/security_audit.md` (audit assumptions, trust boundaries, known limitations)
- `docs/security/front_running_analysis.md` (ordering/MEV-specific analysis)

## System Trust Assumptions

StellarSwipe security depends on the following assumptions:

1. **Stellar network and Soroban runtime are correct**
   - Transaction ordering/finality and `require_auth()` enforcement are trusted.
2. **Privileged keys are protected**
   - Admin/governance/oracle keys are assumed uncompromised.
3. **Oracle quorum is honest enough**
   - Price integrity depends on non-colluding, available oracle operators.
4. **Off-chain operators behave correctly**
   - Frontend/indexer/deployment operators are trusted not to misconfigure addresses, networks, or contract IDs.
5. **Governance participants are economically aligned**
   - Governance controls are assumed to be used for protocol health, not adversarial takeover.

## Privileged Roles and Capabilities

The following roles have privileged powers and should be treated as high-trust:

### 1) Contract Admin

Scope: contract initialization, parameter updates, emergency controls, role assignment.

Capabilities include (varies by contract):
- Initialize contract configuration
- Pause/unpause selected operations
- Update risk/fee configuration
- Configure guardian and multisig signer sets
- Set oracle/trade executor/dependent contract addresses
- Transfer admin (where implemented)

Limitations:
- Bounded by on-chain checks (e.g., fee caps, type validation)
- Some actions are constrained by function-level guards and state checks

### 2) Multisig Signers (when enabled)

Scope: shared authorization for sensitive admin actions.

Capabilities:
- Jointly approve protected administrative operations
- Raise compromise resistance vs single-key admin

Limitations:
- Security depends on threshold/signer distribution quality
- If multisig is not enabled, this protection is inactive

### 3) Guardian / Emergency Operator

Scope: emergency response paths.

Capabilities:
- Trigger or assist emergency pause flows in supported modules
- Reduce blast radius during incident response

Limitations:
- Intended for temporary risk containment, not full protocol governance

### 4) Oracle Operators

Scope: data integrity for price-dependent protocol behavior.

Capabilities:
- Submit prices and influence consensus output
- Affect stop-loss checks, valuation, and execution decisions

Limitations:
- Subject to oracle consistency/deviation checks
- Subject to reputation/slashing logic where implemented

### 5) Governance Actors (token holders/delegates/committees)

Scope: protocol-level governance and treasury operations.

Capabilities:
- Create, vote, and execute governance proposals
- Manage treasury/committee/timelock-configured actions
- Adjust protocol policy through approved governance flows

Limitations:
- Constrained by proposal lifecycle rules and voting mechanics
- Security depends on decentralization and anti-capture assumptions

## Admin Powers and Safety Boundaries

Admin authority is broad and is a central trust point.

What admin can do:
- Change core config parameters
- Register/update key dependency addresses
- Pause selected functionality
- Manage privileged role assignment

What admin should not be assumed able to do:
- Bypass Soroban auth rules without valid signatures
- Arbitrarily modify historical ledger state
- Break protocol invariants that are hard-coded in contract logic

Operational requirement:
- Use hardware-backed keys and multisig before mainnet production governance.

## Oracle Trust Model

StellarSwipe treats oracle input as a high-value trust boundary.

Security intent:
- Reject stale or highly inconsistent data
- Use consensus/reputation mechanisms to reduce single-source corruption risk

Residual trust risk:
- Colluding or compromised oracle quorum can still produce harmful price outcomes
- Oracle outage can degrade availability for price-dependent functions

Users should assume:
- Oracle-integrity risk is reduced, not eliminated
- Higher-value usage requires stronger oracle decentralization and monitoring

## Governance Security Model

Governance is a controlled mechanism for protocol evolution and treasury actions.

Security properties:
- Explicit on-chain proposal and voting flows
- Structured execution pathways for approved actions

Residual risks:
- Governance capture (whale concentration, collusion, or voter apathy)
- Malicious but valid proposals if social/process controls are weak

Operational safeguards recommended:
- Transparent proposal review windows
- Independent security review for high-impact proposals
- Timelock and emergency controls for critical upgrades

## What StellarSwipe Protects Against

Designed protections include:
- Unauthorized state mutation without required signatures
- Basic parameter sanity violations through input checks
- Certain classes of oracle outlier behavior
- Emergency operational controls (pause-oriented containment)

## What StellarSwipe Does Not Fully Protect Against

Out-of-scope or partially mitigated risks include:
- Full key compromise of privileged operators
- Coordinated oracle corruption/collusion
- Governance capture/social attacks
- All economic attacks (e.g., manipulation strategies under adverse market conditions)
- Frontend/indexer/infrastructure misconfiguration or malicious behavior

## Security Review and Sign-off

Required before final release of this model:

- [ ] External security contributor reviewed this document
- [ ] Review feedback incorporated
- [ ] PR contains explicit reviewer sign-off comment

Reviewer sign-off:

| Reviewer | Organization/Handle | Date | Sign-off |
|---|---|---|---|
| TBD | External Security Contributor | — | Pending |
