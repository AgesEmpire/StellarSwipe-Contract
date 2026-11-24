# Liquidity Pool Contract Summary

## Overview
The liquidity pool contract manages pooled reserves and share accounting for
liquidity providers. This document summarizes the contract's administrative
controls, including the pause/unpause mechanism used for emergency actions,
and the price oracle view function.

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
