# Sponsor Recovery of Unused Reward Campaign Funds (Issue #1330)

## Overview
This document specifies how unclaimed and unallocated reward campaign funds are settled after a campaign reaches its terminal state, identifying the authorized sponsor (funder) and detailing how outstanding claims are protected.

---

## Specification & Accounting Model

### 1. Authorized Sponsor Recovery
- **Funder Authorization**: Only the designated campaign funder (`funder`) stored in `RewardCampaign` can recover unused funds when closing the campaign via `close_campaign` / `close_reward_campaign`.
- **Authorization Enforcement**: Contract entry points require authentication from the funder.

### 2. Resolution of Pending Obligations & Claim Window
- **Claim Window Enforced**: Recovery is strictly blocked while any allocation remains `Pending` (`pending_allocations > 0`) or `outstanding > 0`.
- **Explicit Expiry**: Unclaimed allocations past their claim window (`expires_at`) must be explicitly expired via `expire_allocations` before they can transition to the available pool.
- **Terminal State**: Once all allocations are either settled (`Claimed`) or expired (`Expired`), the campaign can be closed (`CampaignStatus::Closed`).

### 3. Derived Refund Amount
- **No Caller Input**: The recovery amount is strictly derived from on-chain tracked campaign accounting:
  ```text
  available = budget - allocated + expired
  ```
- Callers cannot pass arbitrary amounts or request more than the tracked available balance.

### 4. Protection of Valid Claims
- Funds reserved for valid, pending claims (`outstanding > 0`) are fully protected. Any attempt to close the campaign while liabilities exist returns `OutstandingLiabilities` (`CampaignError::OutstandingLiabilities` / `StakeVaultError::CampaignLiabilitiesOutstanding`) without modifying state or releasing funds.

---

## Test Verification
- Tests prove that:
  1. Attempting to recover funds while claims are pending or outstanding fails with `OutstandingLiabilities`.
  2. Recovery amounts are correctly calculated based on budget, allocation, claims, and expiries.
  3. No recovery path can consume funds reserved for valid, active claims.
