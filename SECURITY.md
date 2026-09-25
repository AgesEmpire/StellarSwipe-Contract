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
- Disclosure: 30-60 days

### Extended Timeline

If remediation requires significant architectural changes, we will coordinate an extended timeline with the researcher, keeping them informed of progress at least every 7 days.

---

## Contract State Archival Export

Before any contract upgrade, selected contract state can be exported to a deterministic archival format. This archive is used to inspect state offline and to verify migration completeness after the upgrade.

### Export Record Fields

Every export record MUST include:

- **Contract identity**: the contract ID/address and the network (e.g. `mainnet`, `testnet`) the state was read from.
- **Schema version**: the version of the export format, so archives remain interpretable across tooling changes.
- **Ledger context**: the ledger sequence (and, where available, the ledger close timestamp/hash) at which the state snapshot was taken.
- **State entry**: the storage key and its value, encoded as described below.

### Deterministic Ordering and Encoding

- Records MUST be ordered deterministically by storage key (lexicographic byte order of the encoded key).
- Encoding MUST be canonical: identical state MUST always produce byte-identical archives, independent of iteration order or host environment.
- The archive MUST be self-describing (contract identity, schema version, and ledger context are recorded once per archive and/or per record).

### Handling Sensitive Values

- Sensitive values (private keys, secrets, credentials, or any value classified as confidential) MUST NOT be written to an archive in plaintext.
- Such values MUST be redacted or omitted, and the omission MUST be recorded explicitly so that verification can distinguish "redacted" from "missing".
- Archives MUST be treated as confidential artifacts and stored/transferred according to the repository's security requirements.

### Verification

- Import/verification tooling MUST detect **missing** records (present in the source state but absent from the archive) and **extra** records (present in the archive but absent from the source state).
- A migration is considered complete only when verification reports no missing and no extra records.

---

## Security Researcher Resources

### Documentation

- **Security Best Practices**: `docs/security/best-practices.md`
- **Contract Architecture**: `docs/architecture/`
- **Audit Reports**: `docs/audits/`
- **Threat Model**: `docs/security/threat-model.md`

### Tools

- **Stellar Laboratory**: https://laboratory.stellar.org/
- **Soroban CLI**: https://soroban.stellar.org/docs/getting-started/setup
- **Slither**: Static analysis for Solidity
- **Mythril**: Security analysis tool
- **Foundry**: Testing framework

### Testing Environment

- **Testnet**: Available for security testing
- **Local Development**: See `README.md` for setup
- **Fuzzing**: Encouraged for finding edge cases

---

## Responsible Disclosure Process

### Step-by-Step Process

**1. Discovery**
- Identify potential vulnerability
- Document findings thoroughly
- Prepare proof of concept

**2. Initial Report**
- Submit via preferred channel
- Include all relevant details
- Encrypt sensitive information

**3. Acknowledgment**
- We acknowledge within 48 hours
- Provide tracking ID
- Confirm scope and severity

**4. Investigation**
- We verify the vulnerability
- Assess impact and severity
- Determine remediation approach

**5. Remediation**
- Develop and test fix
- Internal security review
- Deploy to testnet
- Deploy to mainnet

**6. Verification**
- Researcher verifies fix
- Confirm vulnerability resolved
- Discuss disclosure timeline

**7. Disclosure**
- Coordinate public disclosure
- Publish security advisory
- Credit researcher (if desired)
- Pay bounty

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
- Recognize contributions

---

## Legal Safe Harbor

### Our Commitment

StellarSwipe will not pursue legal action against security researchers who:

- ✅ Follow this responsible disclosure policy
- ✅ Act in good faith
- ✅ Avoid privacy violations and data destruction
- ✅ Do not exploit vulnerabilities beyond PoC
- ✅ Report vulnerabilities promptly

### Safe Harbor Conditions

To qualify for safe harbor, researchers must:

1. **Act in Good Faith**
   - Genuine intent to improve security
   - No malicious or profit-seeking motives
   - Reasonable interpretation of this policy

2. **Follow Disclosure Process**
   - Report via designated channels
   - Allow reasonable remediation time
   - Coordinate public disclosure

3. **Minimize Impact**
   - Use testnet when possible
   - Avoid accessing user data
   - Do not disrupt services
   - Do not exfiltrate data

4. **Comply with Laws**
   - Follow applicable laws and regulations
   - Do not violate privacy rights
   - Do not access unauthorized systems

### Limitations

Safe harbor does not apply to:
- ❌ Malicious actors
- ❌ Extortion attempts
- ❌ Violations of law
- ❌ Attacks on production systems
- ❌ Data theft or destruction

### Legal Notice

This policy is not a legal contract and does not create any legally binding obligations. StellarSwipe reserves the right to modify this policy at any time. For legal questions, contact legal@stellarswipe.io.

---

## Contact

**Security Team**: security@stellarswipe.io  
**PGP Key**: `docs/security/pgp-key.asc`  
**GitHub**: https://github.com/AgesEmpire/StellarSwipe-Contract/security/advisories

---

**Last Updated**: 2024  
**Version**: 1.0
