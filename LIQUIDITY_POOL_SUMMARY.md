# Liquidity Pool Contract Summary

## Overview
The liquidity pool contract manages pooled reserves and share accounting for
liquidity providers. This document summarizes the contract's administrative
controls, including the pause/unpause mechanism used for emergency actions.

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
