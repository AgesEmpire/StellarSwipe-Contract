# Liquidity Pool Contract Summary

## Overview
The liquidity pool contract manages pooled reserves and share accounting for
liquidity providers. This document summarizes the contract's administrative
controls, including the pause/unpause mechanism used for emergency actions,
the price oracle view function, and the liquidity deposit flow.

## Administrative Controls

### Pause / Unpause
- **Authorization:** Only the authorized admin may pause or unpause the
  contract. Calls from any other address are rejected with an explicit
  authorization error.
- **Paused behavior:** While the contract is paused, restricted actions
  (trading and deposits) fail with explicit error messages. Read-only and
  accounting queries remain available.
- **Resume behavior:** Unpausing restores normal operation. Reserve balances
  and share accounting are preserved across pause/unpause transitions; no
  state is reset.

## State Integrity
Pause and unpause transitions are state-preserving. They do not modify
reserves, shares, or any accounting values, ensuring that resuming the
contract continues from the exact state prior to pausing.

## Liquidity Deposit

### `deposit(amount_a, amount_b)`
- **Purpose:** Allows a liquidity provider to deposit a pair of token A and
  token B amounts into the pool and receive LP tokens representing their share
  of the pool.
- **Inputs:** `amount_a` and `amount_b` are the quantities of token A and
  token B the depositor contributes. Both amounts must be greater than zero.
- **Initial deposit:** When the pool has no existing liquidity, the LP tokens
  minted equal the geometric mean of the deposited amounts
  (`sqrt(amount_a * amount_b)`), establishing the initial share supply.
- **Proportional minting:** When liquidity already exists, LP tokens are minted
  proportionally to the depositor's share of the pool:
  `lp_minted = min(amount_a * total_lp / reserve_a, amount_b * total_lp / reserve_b)`.
  This ensures the deposit does not dilute existing providers.
- **Reserve updates:** On success, the token A and token B reserves are
  increased by the deposited amounts and the total LP supply is increased by
  the minted amount, all persisted in contract storage.
- **Paused behavior:** Deposits are a restricted action and fail with an
  explicit error while the contract is paused.
- **Tests:** Unit tests verify that LP tokens are minted proportionally to the
  deposited amounts and that pool reserves and total LP supply are updated
  correctly in storage after each deposit.

## Price Oracle

### `get_price()`
- **Purpose:** Read-only view function that returns the current spot price of
  token A in terms of token B, derived directly from the pool reserves.
- **Calculation:** The price is the ratio of the token B reserve to the token A
  reserve (`reserve_b / reserve_a`), i.e. how many units of token B one unit of
  token A is worth at the current reserve levels.
- **No liquidity:** If either reserve is zero (the pool has no liquidity), the
  call returns an explicit error instead of a price, avoiding division by zero
  and meaningless quotes.
- **Availability:** As a read-only query, `get_price()` remains callable while
  the contract is paused.
- **Tests:** Unit tests validate the returned price against known reserve
  values and assert the error path for an empty pool.
