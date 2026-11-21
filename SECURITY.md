# Security Policy

## Reporting a Vulnerability

**Please do not open public issues for security vulnerabilities.**

Report vulnerabilities privately through GitHub's private vulnerability reporting:

👉 https://github.com/AgesEmpire/StellarSwipe-Contract/security/advisories/new

(Repository → **Security** tab → **Report a vulnerability**.)

Please include:

- Affected contract(s) and function(s)
- Severity assessment and impact
- Steps to reproduce / proof of concept
- Suggested remediation, if any

## Response Process

| Stage | Target |
|-------|--------|
| Acknowledgement | within 72 hours |
| Initial triage & severity rating | within 7 days |
| Fix / mitigation plan | depends on severity |
| Public disclosure | coordinated with the reporter after a fix ships |

Reporters are credited in the published advisory unless they ask otherwise.

## Scope

All Soroban contracts under `stellar-swipe/contracts/`. Frontend and scripts are
in scope only when they can lead to loss of funds or unauthorized contract calls.

## Maintainer Note

Private vulnerability reporting must stay enabled under
**Settings → Code security → Private vulnerability reporting**.
No email, Keybase, or bug bounty channel is currently active; this advisory
form is the only supported private channel.
