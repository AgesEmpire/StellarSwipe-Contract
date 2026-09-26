# Security Policy

## Security Vulnerability Disclosure Program

StellarSwipe is committed to ensuring the security of our smart contract platform and protecting our users' assets. We welcome the security research community to help us maintain the highest security standards.

---

## Table of Contents

1. [Vulnerability Disclosure Policy](#vulnerability-disclosure-policy)
2. [Responsible Disclosure Channels](#responsible-disclosure-channels)
3. [Bug Bounty Program](#bug-bounty-program)
4. [Disclosure Timeline](#disclosure-timeline)
5. [Security Researcher Resources](#security-researcher-resources)
6. [Responsible Disclosure Process](#responsible-disclosure-process)
7. [Legal Safe Harbor](#legal-safe-harbor)
8. [Contract State Archival Export](#contract-state-archival-export)
9. [Governance Timelock Exceptions](#governance-timelock-exceptions)

---

## Vulnerability Disclosure Policy

### Our Commitment

We are committed to working with security researchers to:
- Quickly verify and respond to legitimate vulnerability reports
- Keep researchers informed throughout the remediation process
- Recognize researchers who help improve our security
- Maintain transparency with our community

### Scope

**In Scope:**
- All smart contracts in this repository
- Contract deployment and upgrade mechanisms
- Access control and authorization systems
- Token handling and transfer logic
- Reward distribution mechanisms
- Staking and vault systems
- Fee collection and treasury management
- Oracle integrations (if applicable)
- Frontend integrations that affect contract security

**Out of Scope:**
- Third-party services and dependencies
- Social engineering attacks
- Physical security
- Denial of service attacks on public networks
- Issues already publicly disclosed
- Issues in deprecated or archived code

### Vulnerability Categories

We are particularly interested in vulnerabilities related to:

**Critical:**
- Unauthorized fund access or theft
- Contract upgrade/takeover vulnerabilities
- Privilege escalation to admin/owner
- Reentrancy attacks leading to fund loss
- Integer overflow/underflow causing fund loss
- Flash loan attacks with economic impact

**High:**
- Access control bypass
- Logic errors causing incorrect state
- Front-running vulnerabilities with significant impact
- Oracle manipulation
- Reward calculation errors
- Improper input validation leading to exploits

**Medium:**
- Information disclosure
- Denial of service (contract level)
- Gas optimization issues with security implications
- Timestamp manipulation
- Rounding errors with minor economic impact

**Low:**
- Best practice violations
- Code quality issues with potential security implications
- Documentation errors that could lead to misuse

---

## Responsible Disclosure Channels

### Primary Contact Methods

**1. Security Email (Preferred)**
- **Email**: security@stellarswipe.io
- **PGP Key**: Available at `docs/security/pgp-key.asc`
- **Response Time**: Within 48 hours

**2. Bug Bounty Platform**
- **Platform**: [To be announced]
- **URL**: [To be announced]
- **For**: Structured submissions with bounty eligibility

**3. Private GitHub Security Advisory**
- **URL**: https://github.com/AgesEmpire/StellarSwipe-Contract/security/advisories
- **For**: Detailed technical reports with code references

**4. Encrypted Communication**
- **Keybase**: stellarswipe_security
- **Signal**: [To be announced]
- **For**: Sensitive or time-critical disclosures

### What to Include in Your Report

Please provide as much information as possible:

1. **Vulnerability Description**
   - Clear description of the vulnerability
   - Affected components/contracts
   - Vulnerability category

2. **Impact Assessment**
   - Potential impact on users and protocol
   - Estimated severity (Critical/High/Medium/Low)
   - Attack scenarios

3. **Proof of Concept**
   - Step-by-step reproduction instructions
   - Code snippets or test cases
   - Transaction examples (testnet preferred)

4. **Suggested Fix** (Optional)
   - Proposed remediation approach
   - Code patches or recommendations

5. **Researcher Information**
   - Name/Handle (for attribution)
   - Contact information
   - Ethereum/Stellar address (for bounty payments)

### Communication Guidelines

**DO:**
- ✅ Use encrypted channels for sensitive information
- ✅ Provide detailed technical information
- ✅ Allow reasonable time for response and remediation
- ✅ Keep the vulnerability confidential until disclosure
- ✅ Work with us to understand the full impact

**DON'T:**
- ❌ Publicly disclose before coordinated disclosure date
- ❌ Exploit the vulnerability beyond proof of concept
- ❌ Access or modify user data
- ❌ Perform attacks on mainnet
- ❌ Demand payment before disclosure

---

## Bug Bounty Program

### Reward Tiers

Bounty rewards are determined by severity and impact:

#### Critical Severity
**Reward: $10,000 - $50,000 USD (or equivalent in XLM)**

Examples:
- Direct theft of user funds
- Permanent freezing of funds
- Protocol insolvency
- Unauthorized contract upgrade
- Complete access control bypass

#### High Severity
**Reward: $5,000 - $10,000 USD (or equivalent in XLM)**

Examples:
- Theft of unclaimed yield/rewards
- Temporary freezing of funds
- Privilege escalation
- Significant logic errors
- Oracle manipulation with economic impact

#### Medium Severity
**Reward: $1,000 - $5,000 USD (or equivalent in XLM)**

Examples:
- Griefing attacks (no profit motive)
- Smart contract gas manipulation
- Minor access control issues
- Information disclosure
- Reward calculation errors

#### Low Severity
**Reward: $100 - $1,000 USD (or equivalent in XLM)**

Examples:
- Best practice violations
- Code quality issues
- Documentation errors
- Minor optimization opportunities

### Bounty Eligibility

**Eligible:**
- ✅ First reporter of a unique vulnerability
- ✅ Vulnerabilities in current production code
- ✅ Clear proof of concept provided
- ✅ Followed responsible disclosure process
- ✅ Allowed reasonable remediation time

**Not Eligible:**
- ❌ Duplicate reports
- ❌ Publicly known issues
- ❌ Out of scope vulnerabilities
- ❌ Issues in test/development code
- ❌ Theoretical vulnerabilities without PoC
- ❌ Violations of disclosure policy

### Bounty Determination Factors

Rewards are determined based on:

1. **Severity**: Impact on users and protocol
2. **Quality**: Clarity and completeness of report
3. **Exploitability**: Ease of exploitation
4. **Impact**: Number of users/funds affected
5. **Creativity**: Novel attack vectors
6. **Cooperation**: Researcher's collaboration during remediation

### Payment Process

1. **Verification**: We verify the vulnerability (1-5 business days)
2. **Assessment**: Severity and bounty amount determined (2-5 business days)
3. **Notification**: Researcher notified of bounty decision
4. **Remediation**: Fix developed and deployed
5. **Payment**: Bounty paid after fix verification
6. **Disclosure**: Coordinated public disclosure (optional)

**Payment Methods:**
- XLM (Stellar Lumens) - Preferred
- USDC on Stellar
- Bank transfer (for amounts >$5,000)
- Cryptocurrency (BTC, ETH) upon request

---

## Disclosure Timeline

### Standard Timeline

We follow a responsible disclosure timeline to protect users while maintaining transparency:

**Day 0: Report Received**
- Acknowledge receipt within 48 hours
- Assign tracking ID
- Begin initial assessment

**Day 1-5: Verification**
- Verify vulnerability
- Assess severity and impact
- Determine bounty eligibility
- Communicate findings to researcher

**Day 5-30: Remediation**
- Develop fix
- Internal testing
- Security review
- Prepare deployment plan

**Day 30-45: Deployment**
- Deploy fix to testnet
- Monitor for issues
- Deploy to mainnet
- Verify fix effectiveness

**Day 45-90: Public Disclosure**
- Coordinate disclosure date with researcher
- Prepare public advisory
- Publish security update
- Credit researcher (if desired)

### Expedited Timeline

For **Critical** vulnerabilities:
- Verification: 24-48 hours
- Remediation: 5-14 days
- Deployment: Immediate after testing

---

## Governance Timelock Exceptions

The `contracts/governance/` contract enforces an execution delay (timelock) for critical governance actions. Every state-mutating entry point in `governance/src/lib.rs` is categorized below. Category (a) functions are gated behind `timelock::require_passed`; category (b) functions have been routed through the timelock queue; category (c) functions are intentionally ungated and documented here with rationale.

### Category (a) — Correctly Gated

These entry points require a queued action whose delay has elapsed before they execute:

- `set_treasury_address` — treasury redirection is a high-impact action; must be queued and delayed.
- `update_token` — changing the governance token affects voting power; must be queued and delayed.
- `add_committee_member` / `remove_committee_member` — committee membership changes alter governance control; must be queued and delayed.
- `update_quorum` / `update_voting_period` — parameter changes affecting proposal outcomes; must be queued and delayed.

### Category (b) — Routed Through the Timelock

Any previously ungated state-mutating entry point that modifies critical governance state has been routed through the timelock queue: the action is queued, the delay is enforced via `timelock::require_passed`, and only then is it executed. Direct calls that skip queueing are rejected.

### Category (c) — Intentional Exceptions

The following entry points are intentionally **not** gated by the timelock. Each exception is deliberate and justified:

- **Emergency pause / unpause** — Must be callable immediately to halt the protocol in response to an active exploit. A timelock delay would defeat the purpose of an emergency stop. Access is restricted to the emergency admin role, and unpausing is expected to be followed by a governance review.
- **Timelock queueing itself** (`queue`) — Queueing an action is the mechanism that *starts* the delay; it cannot itself be delayed. It is access-controlled to authorized governance callers.
- **Timelock cancellation** (`cancel`) — Cancelling a pending action is a safety mechanism to abort a malicious or erroneous queued action before it executes. It is access-controlled and does not modify critical protocol state directly.
- **Read-only / view functions** — Functions that do not mutate state (e.g., `get_timelock_delay`, `is_action_ready`) are not gated because they cannot alter protocol state.

### Rationale

Timelocks are the primary protection against sudden malicious governance actions. A single ungated function that mutates critical state would make the delay theater. The exceptions above are limited to (1) emergency response that must be immediate, (2) the timelock's own queue/cancel machinery, and (3) read-only accessors. All other state-mutating entry points are gated or routed through the timelock, and tests verify that direct calls to timelocked functions without queueing are rejected.

---

## Security Researcher Resources

### Tools and Resources

**Recommended Tools:**
- Soroban CLI
- Stellar Laboratory
- Rust Analyzer
- Cargo Audit
- Slither (for Solidity, if applicable)

**Documentation:**
- [Stellar Smart Contracts](https://developers.stellar.org/docs/smart-contracts)
- [Soroban Documentation](https://soroban.stellar.org/docs)
- [Security Best Practices](https://developers.stellar.org/docs/smart-contracts/security)

### Testing Environment

**Testnet Access:**
- Network: Stellar Testnet
- RPC: https://soroban-testnet.stellar.org
- Faucet: https://laboratory.stellar.org/#account-creator

**Local Testing:**
```bash
# Clone repository
git clone https://github.com/AgesEmpire/StellarSwipe-Contract.git
cd StellarSwipe-Contract

# Run tests
cargo test

# Build contracts
cargo build --target wasm32-unknown-unknown --release
```

---

## Responsible Disclosure Process

### Step-by-Step Process

**1. Discovery**
- Identify potential vulnerability
- Document findings
- Create proof of concept

**2. Initial Report**
- Submit via preferred channel
- Include all required information
- Encrypt if sensitive

**3. Acknowledgment**
- Receive confirmation within 48 hours
- Get tracking ID
- Establish communication channel

**4. Verification**
- We verify the vulnerability
- May request additional information
- Assess severity and impact

**5. Remediation**
- We develop and test fix
- Keep you informed of progress
- May request your input on fix

**6. Deployment**
- Deploy fix to testnet
- Verify fix effectiveness
- Deploy to mainnet

**7. Recognition**
- Bounty payment (if eligible)
- Public credit (if desired)
- Hall of fame listing

**8. Disclosure**
- Coordinate public disclosure
- Publish security advisory
- Share lessons learned

### Communication Expectations

**From Researchers:**
- Provide clear, detailed reports
- Respond to questions promptly
- Allow reasonable remediation time
- Maintain confidentiality

**From StellarSwipe:**
- Acknowledge reports within 48 hours
- Provide regular updates
- Be transparent about remediation
- Recognize contributions fairly

---

## Legal Safe Harbor

### Our Commitment

StellarSwipe will not pursue legal action against security researchers who:

- ✅ Act in good faith
- ✅ Follow this disclosure policy
- ✅ Do not exploit vulnerabilities beyond PoC
- ✅ Do not access or modify user data
- ✅ Do not perform attacks on mainnet
- ✅ Allow reasonable remediation time
- ✅ Maintain confidentiality until disclosure

### Conditions

Safe harbor applies when researchers:

1. **Act in Good Faith**
   - Genuine intent to improve security
   - No malicious intent
   - No personal gain beyond bounty

2. **Follow Policy**
   - Use approved disclosure channels
   - Provide reasonable remediation time
   - Do not publicly disclose prematurely

3. **Minimize Impact**
   - Do not exploit beyond PoC
   - Do not access user data
   - Do not disrupt services

4. **Cooperate**
   - Provide detailed information
   - Respond to questions
   - Work with us on remediation

### Limitations

Safe harbor does **not** apply to:

- ❌ Malicious actors
- ❌ Those who violate this policy
- ❌ Those who exploit for personal gain
- ❌ Those who access user data
- ❌ Those who perform mainnet attacks
- ❌ Those who publicly disclose prematurely

### Legal Disclaimer

This safe harbor statement is not a legal contract and does not override any applicable laws. It represents our intent to work cooperatively with security researchers. For legal questions, contact legal@stellarswipe.io.

---

## Contract State Archival Export

### Overview

Soroban contracts on Stellar have a state archival mechanism where contract data can be archived if not accessed for a period of time. StellarSwipe contracts implement state archival export functionality to ensure critical state can be recovered.

### Archival Export Process

**1. State Snapshot**
- Contract state is periodically snapshotted
- Snapshots include all critical state variables
- Snapshots are stored off-chain in encrypted form

**2. Export Mechanism**
- Admin can trigger state export
- Export includes: balances, stakes, rewards, governance state
- Export is signed and timestamped

**3. Recovery Process**
- If state is archived, use export to restore
- Verify export integrity
- Restore state to new contract instance

### Security Considerations

- Export data is encrypted at rest
- Access to exports is restricted to admin
- Export operations are logged
- Recovery requires multi-sig approval

### For Researchers

If you discover issues with state archival or export:
- Test on testnet only
- Do not attempt to access production exports
- Report via standard disclosure channels
- Include impact assessment

---

## Contact

**Security Team:** security@stellarswipe.io

**PGP Key:** docs/security/pgp-key.asc

**Bug Bounty:** [To be announced]

**GitHub Security Advisory:** https://github.com/AgesEmpire/StellarSwipe-Contract/security/advisories

---

*Last Updated: 2024*

*This security policy is subject to change. Check back regularly for updates.*
