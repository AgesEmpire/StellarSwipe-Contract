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

If remediation requires significant changes:
- We will communicate delays to researcher
- Provide regular status updates
- Agree on extended timeline
- May provide interim mitigations

### Early Disclosure

We may disclose earlier if:
- Vulnerability is being actively exploited
- Information has been leaked publicly
- Researcher agrees to early disclosure
- Users are at immediate risk

---

## Security Researcher Resources

### Testing Environments

**Testnet Deployment**
- **Network**: Stellar Testnet
- **Contracts**: [Testnet addresses in deployments/testnet.json]
- **Faucet**: https://friendbot.stellar.org
- **Explorer**: https://stellar.expert/explorer/testnet

**Local Development**
- **Setup Guide**: `docs/development.md`
- **Test Suite**: `cargo test`
- **Soroban CLI**: https://soroban.stellar.org/docs

### Documentation

**Technical Documentation**
- Architecture: `docs/architecture.md`
- Contract Specs: `docs/contracts/`
- Security Model: `docs/security/security_model.md`
- Threat Model: `docs/security/threat_model.md`

**Security Analyses**
- Reentrancy: `docs/security/reentrancy_analysis.md`
- Access Control: `docs/security/privilege_escalation_analysis.md`
- Flash Loans: `docs/security/flash_loan_analysis.md`
- Front-running: `docs/security/front_running_analysis.md`

**Previous Audits**
- [Audit reports will be published in `docs/audits/`]

### Tools and Resources

**Recommended Tools:**
- Soroban CLI for contract interaction
- Stellar Laboratory for transaction building
- Rust analyzer for code review
- Foundry/Hardhat for testing (if applicable)

**Useful Resources:**
- Soroban Documentation: https://soroban.stellar.org
- Stellar Documentation: https://developers.stellar.org
- Smart Contract Security Best Practices
- Common vulnerability patterns

### Code Review Focus Areas

When reviewing our code, pay special attention to:

1. **Access Control**
   - Admin functions and privilege checks
   - Role-based access control
   - Ownership transfer mechanisms

2. **Asset Handling**
   - Token transfers and approvals
   - Balance calculations
   - Reward distributions

3. **State Management**
   - Storage key collisions
   - State consistency
   - Upgrade safety

4. **External Calls**
   - Reentrancy protection
   - Return value handling
   - Gas limits

5. **Math Operations**
   - Overflow/underflow protection
   - Rounding errors
   - Precision loss

6. **Time Dependencies**
   - Timestamp manipulation
   - Block number dependencies
   - Deadline enforcement

---

## Responsible Disclosure Process

### Step-by-Step Process

#### 1. Discovery
- Identify potential vulnerability
- Verify on testnet if possible
- Document findings thoroughly

#### 2. Initial Report
- Submit via secure channel
- Include all required information
- Use provided report template

#### 3. Acknowledgment
- Receive confirmation (within 48 hours)
- Get assigned tracking ID
- Establish communication channel

#### 4. Verification
- Team verifies vulnerability
- May request additional information
- Severity assessment conducted

#### 5. Bounty Decision
- Eligibility determined
- Reward amount communicated
- Payment timeline provided

#### 6. Remediation
- Fix developed and tested
- Researcher may be consulted
- Regular status updates provided

#### 7. Deployment
- Fix deployed to production
- Verification of effectiveness
- Monitoring for issues

#### 8. Disclosure
- Coordinate disclosure date
- Prepare public advisory
- Publish security update
- Credit researcher

#### 9. Payment
- Bounty payment processed
- Receipt confirmation
- Thank you and recognition

### Report Template

```markdown
# Security Vulnerability Report

## Basic Information
- **Reporter**: [Your name/handle]
- **Contact**: [Email/Keybase/Signal]
- **Date**: [YYYY-MM-DD]
- **Severity**: [Critical/High/Medium/Low]

## Vulnerability Summary
[Brief description of the vulnerability]

## Affected Components
- Contract: [Contract name/address]
- Function: [Affected function(s)]
- Version: [Commit hash or version]

## Vulnerability Details
[Detailed technical description]

## Impact Assessment
- **Users Affected**: [Number/percentage]
- **Funds at Risk**: [Amount/percentage]
- **Attack Complexity**: [Low/Medium/High]
- **Prerequisites**: [Required conditions]

## Proof of Concept
[Step-by-step reproduction]

```rust
// Code example
```

## Suggested Fix
[Optional: Your recommendations]

## Additional Information
[Any other relevant details]

## Payment Address
- **Stellar Address**: [Your address for bounty]
```

### Communication Expectations

**From Us:**
- Initial response within 48 hours
- Status updates every 7 days minimum
- Clear timeline for remediation
- Transparent bounty decision process
- Respectful and professional communication

**From Researchers:**
- Confidentiality until coordinated disclosure
- Cooperation during verification
- Reasonable response times
- Professional communication
- Patience during remediation

---

## Legal Safe Harbor

### Safe Harbor Provision

StellarSwipe commits to not pursuing legal action against security researchers who:

1. **Act in Good Faith**
   - Make a good faith effort to comply with this policy
   - Do not intentionally harm users or the protocol
   - Report vulnerabilities promptly

2. **Respect Boundaries**
   - Only test on testnet or local environments
   - Do not access or modify user data
   - Do not exploit vulnerabilities beyond PoC
   - Do not perform attacks on mainnet

3. **Maintain Confidentiality**
   - Keep vulnerability details confidential
   - Follow coordinated disclosure timeline
   - Do not publicly disclose prematurely

4. **Comply with Laws**
   - Follow all applicable laws and regulations
   - Respect intellectual property rights
   - Do not violate terms of service

### What is Protected

Under this safe harbor:
- Security research activities on testnet
- Vulnerability analysis of public code
- Proof of concept development
- Responsible disclosure communications
- Good faith security testing

### What is NOT Protected

This safe harbor does NOT protect:
- Attacks on mainnet or production systems
- Theft or destruction of data
- Intentional harm to users
- Violations of law
- Social engineering attacks
- Physical security testing
- Attacks on third-party systems

### Disclaimer

This policy is not a contract and does not create legal rights. We reserve the right to modify this policy at any time. However, we commit to honoring the spirit of this policy for all good faith security research.

---

## Recognition

### Hall of Fame

We maintain a security researcher hall of fame to recognize contributors:
- `docs/security/hall_of_fame.md`

Recognition includes:
- Name/handle (with permission)
- Vulnerability discovered
- Severity level
- Date of disclosure

### Public Acknowledgment

With your permission, we will:
- Credit you in security advisories
- Mention you in release notes
- Feature you in our hall of fame
- Share your research (after disclosure)

You may choose to:
- Remain anonymous
- Use a pseudonym
- Decline public recognition

---

## Contact Information

### Security Team

**Primary Contact:**
- Email: security@stellarswipe.io
- PGP: See `docs/security/pgp-key.asc`

**Emergency Contact:**
- For critical vulnerabilities requiring immediate attention
- Email: emergency-security@stellarswipe.io

**Bug Bounty Platform:**
- [To be announced]

**Social Media:**
- Twitter: @StellarSwipe
- Discord: [Community server]
- Telegram: [Security channel]

### Response Times

- **Critical**: 24 hours
- **High**: 48 hours
- **Medium**: 72 hours
- **Low**: 5 business days

---

## Updates and Changes

This security policy may be updated periodically. Changes will be:
- Announced via our communication channels
- Documented in version history
- Applied to new reports only (existing reports follow original terms)

**Last Updated**: 2026-06-01  
**Version**: 1.0.0

---

## Frequently Asked Questions

### Q: Can I test on mainnet?
**A:** No. All testing should be done on testnet or local environments. Mainnet testing is not authorized and may result in legal action.

### Q: How long should I wait before public disclosure?
**A:** Please wait for our coordinated disclosure date, typically 45-90 days after report. We will work with you to agree on a timeline.

### Q: What if I disagree with the severity assessment?
**A:** We welcome discussion. Please provide additional context or impact analysis, and we will reconsider.

### Q: Can I report anonymously?
**A:** Yes, but you must provide a way for us to contact you and verify your identity for bounty payment.

### Q: What if the vulnerability is already being exploited?
**A:** Report immediately via emergency contact. We will expedite response and may disclose earlier to protect users.

### Q: Do you accept reports from automated tools?
**A:** Yes, but please verify findings and provide context. Automated tool output alone is not sufficient.

### Q: Can I share my findings with others?
**A:** Not until after coordinated public disclosure. Sharing prematurely violates the disclosure policy.

### Q: What if I accidentally discover a vulnerability while using the platform?
**A:** Please report it! Accidental discovery is fine as long as you don't exploit it and report promptly.

---

**Thank you for helping keep StellarSwipe secure!**

For questions about this policy, contact: security@stellarswipe.io
