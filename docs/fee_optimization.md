# Fee Optimization Methodology

Stellar Swipe uses a dynamic fee model that combines:

- A base fee configured by protocol governance.
- User-specific volume rebates for silver and gold trading tiers.
- Network condition adjustments based on congestion signals.
- Protocol token payment discounts when eligible.

## Fee calculation flow

1. Fetch the user-specific base fee rate, including volume-based rebate tiers.
2. Apply a network condition premium based on the current on-chain network score.
3. Cap the result at the configured dynamic fee maximum.
4. Apply protocol token discounts when the payment asset matches the configured protocol token.

## Goals

- Maintain fee predictability while adapting to market and network conditions.
- Encourage high-volume traders with tiered discounts.
- Provide transparent, auditable fee reports to users and operators.
