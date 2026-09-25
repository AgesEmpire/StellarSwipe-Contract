# Arithmetic Invariants Proptest Harness

## Overview

This document describes the property-based testing harness for fee, PnL, and liquidity-pool share arithmetic invariants in the StellarSwipe contracts. The harness uses `proptest` to generate randomized valid input ranges and asserts core arithmetic invariants hold for fee-splitting calculations in `fee_collector`, realized-PnL calculations in `trade_executor`, and share mint/burn conversions in the liquidity pool.

## Location

The test harness is located at:
```
stellar-swipe/contracts/integration_tests/tests/integration/test_arithmetic_invariants.rs
```

## Running the Tests

### Prerequisites

Ensure you have Rust and the Soroban toolchain installed:
```bash
rustup install stable
rustup component add rust-src
```

### Run All Arithmetic Invariant Tests

```bash
cd stellar-swipe/contracts/integration_tests
cargo test --test test_arithmetic_invariants
```

### Run Specific Test

```bash
cargo test --test test_arithmetic_invariants prop_fee_calculation_no_overflow
```

### Run with Verbose Output

```bash
cargo test --test test_arithmetic_invariants -- --nocapture
```

## Invariants Tested

### Fee Splitting Invariants (fee_collector)

The fee splitting logic in `fee_collector` is tested for the following invariants:

1. **No Arithmetic Overflow**: Fee calculation never overflows for valid input ranges
   - Input ranges: trade amounts (1 to 10^12), fee rates (1-100 bps)
   - Ensures `fee_amount_floor` returns `Some` for all valid inputs

2. **Fee Conservation**: `fee + net_amount = trade_amount`
   - Verifies that the fee charged plus the amount the trader receives equals the original trade amount
   - Ensures no dust is lost or created

3. **Fee Bounds**: Fee is non-negative and does not exceed trade amount
   - `0 <= fee <= trade_amount`
   - Fundamental sanity check for fee calculations

4. **Burn Amount No Overflow**: Burn amount calculation never overflows
   - `burn_amount = fee_amount * burn_rate_bps / 10_000`
   - Tested for burn rates from 0 to 10,000 bps (0-100%)

5. **Burn Conservation**: `burn + distributable = fee`
   - Ensures no dust accumulates from burn calculations
   - Critical for treasury accounting

6. **Burn Amount Bounded**: Burn amount is bounded by fee amount
   - `0 <= burn_amount <= fee_amount`
   - Prevents burning more than the collected fee

7. **Referral Share No Overflow**: Referral share calculation never overflows
   - `referral_amount = fee_amount * referral_rate_bps / 10_000`
   - Tested for referral rates up to 5,000 bps (50%)

8. **Revenue Share No Overflow**: Revenue share calculation never overflows
   - `revenue_amount = distributable * revenue_rate_bps / 10_000`
   - Tested for revenue rates up to 5,000 bps (50%)

9. **Fee Components Sum to Total**: `burn + referral + revenue_share + treasury = fee`
   - Tests the complete fee distribution logic
   - Ensures all fee components are accounted for
   - Includes capping logic (referral and revenue share capped at available distributable)

10. **Fee Monotonic with Rate**: Higher fee rate yields equal or higher fee
    - Ensures fee calculation is monotonic in the fee rate
    - Prevents perverse incentives

11. **Fee Monotonic with Amount**: Higher trade amount yields equal or higher fee
    - Ensures fee calculation is monotonic in the trade amount
    - Prevents perverse incentives

### PnL Calculation Invariants (trade_executor)

The realized-PnL calculation logic in `trade_executor` is tested for the following invariants:

1. **Entry Value No Overflow**: Entry value calculation never overflows for realistic inputs
   - `entry_value = amount * entry_price / PRICE_PRECISION`
   - PRICE_PRECISION = 10,000,000 (7 decimals, Stellar standard)
   - Tested for amounts up to 10^12 and prices up to 10^13

2. **Entry Value Non-Negative**: Entry value is always non-negative
   - `entry_value >= 0`
   - Fundamental sanity check

3. **Realized PnL No Panic**: PnL calculation never panics (uses saturating arithmetic)
   - `realized_pnl = exit_price - entry_value`
   - Uses saturating subtraction to prevent underflow

4. **Realized PnL Conservative (Loss)**: When exit_price < entry_value, PnL is non-positive
   - Ensures losses are correctly calculated
   - Prevents overestimation of profit

5. **Realized PnL Conservative (Profit)**: When exit_price >= entry_value, PnL is non-negative
   - Ensures profits are correctly calculated
   - Prevents underestimation of profit

6. **PnL Composition Conservative**: Combined PnL equals sum of individual PnLs
   - For multiple trades: `total_pnl = sum(individual_pnls)`
   - Ensures PnL is additive and conservative under composition

7. **Entry Value Precision Bounds**: Entry value respects asset precision bounds
   - `entry_value <= amount * max_price`
   - Ensures calculations stay within realistic bounds

### Liquidity Pool Share Invariants (liquidity_pool)

The share mint/burn conversions in the liquidity pool are tested for the following invariants. Every conversion between assets and shares has a documented rounding direction; the pool always rounds in favor of the pool (never the depositor/withdrawer) so that rounding drift cannot be farmed across repeated operations.

**Rounding direction for every conversion:**

| Conversion | Formula | Rounding | Rationale |
| --- | --- | --- | --- |
| Deposit → shares minted | `shares = deposit * total_shares / total_assets` | **floor** | Never mint more shares than the deposit is worth; any dust stays in the pool. |
| Withdraw → assets returned | `assets = shares * total_assets / total_shares` | **floor** | Never pay out more assets than the shares are worth; any dust stays in the pool. |
| Initial deposit → shares minted | `shares = deposit` (1:1 bootstrap) | **exact** | First deposit defines the share/asset ratio; no rounding is possible. |
| Minimum-share guard | `shares >= MIN_LIQUIDITY_SHARES` | **ceil check** | Reject deposits that would mint fewer than the minimum, preventing zero-share mints. |

1. **Share Mint No Overflow**: Share minting never overflows for valid input ranges
   - `shares = deposit * total_shares / total_assets`
   - Tested for deposits up to 10^12 and pool sizes up to 10^18

2. **Share Mint Rounds Down**: Minted shares never exceed the exact rational value
   - `shares * total_assets <= deposit * total_shares`
   - Guarantees the pool is never diluted by a deposit

3. **Share Burn Rounds Down**: Assets returned never exceed the exact rational value
   - `assets * total_shares <= shares * total_assets`
   - Guarantees the pool is never drained by a withdrawal

4. **Conservation Across Repeated Deposits**: For a sequence of deposits, the sum of minted shares equals the shares computed from the aggregate deposit
   - `sum(mint(d_i)) <= mint(sum(d_i))`
   - Rounding dust accumulates in the pool, never in the depositor's favor

5. **Conservation Across Repeated Withdrawals**: For a sequence of withdrawals, the sum of returned assets never exceeds the assets computed from the aggregate withdrawal
   - `sum(burn(s_i)) <= burn(sum(s_i))`
   - Rounding dust accumulates in the pool, never in the withdrawer's favor

6. **Round-Trip No Value Creation**: Deposit then immediately withdraw never returns more assets than deposited
   - `burn(mint(deposit)) <= deposit`
   - Prevents a deposit/withdraw cycle from creating value out of rounding

7. **Minimum-Share Invariant**: A deposit that would mint fewer than `MIN_LIQUIDITY_SHARES` shares is rejected
   - `mint(deposit) >= MIN_LIQUIDITY_SHARES` for every accepted deposit
   - Prevents tiny deposits from minting zero shares or being used to grief the pool

8. **Zero-Share Guard**: A deposit that would mint zero shares is rejected rather than silently accepted
   - `mint(deposit) == 0` is an error, not a no-op
   - Prevents share-supply manipulation via dust deposits

9. **Monotonic in Deposit**: Larger deposits mint equal or more shares
   - `deposit_a <= deposit_b => mint(deposit_a) <= mint(deposit_b)`
   - Prevents perverse incentives

10. **Monotonic in Shares Burned**: Burning more shares returns equal or more assets
    - `shares_a <= shares_b => burn(shares_a) <= burn(shares_b)`
    - Prevents perverse incentives

## Input Constraints

All generated inputs are constrained to realistic, valid ranges reflecting real contract usage:

### Fee Splitting
- **Trade amounts**: 1 to 10^12 (no negative balances)
- **Fee rates**: 1 to 100 bps (0.01% to 1%)
- **Burn rates**: 0 to 10,000 bps (0% to 100%)
- **Referral/revenue share rates**: 0 to 5,000 bps (0% to 50%)

### PnL Calculation
- **Trade amounts**: 1 to 10^12
- **Entry/exit prices**: 1 to 10^13 (7-decimal precision, Stellar standard)
- **Entry values**: 0 to 10^18 (realistic bound for amount * price)

### Liquidity Pool Shares
- **Deposits**: 1 to 10^12 (tiny amounts included to exercise the minimum-share guard)
- **Total assets**: 1 to 10^18
- **Total shares**: 1 to 10^18
- **Shares burned**: 1 to 10^18
- **Boundary cases**: `i128::MAX` and `i128::MIN` inputs to confirm checked arithmetic rejects overflow instead of wrapping

## Test Configuration

The harness uses the following proptest configuration:
```rust
ProptestConfig {
    cases: 1000,  // Reasonable budget for CI
    ..ProptestConfig::default()
}
```

This generates 1,000 test cases per property, providing good coverage while keeping CI runtime reasonable.

## Integration Tests

In addition to property-based tests, the harness includes end-to-end integration tests:

1. **Fee Collection End-to-End**: Tests fee collection with realistic parameters
   - Verifies invariants hold through the full fee collection flow
   - Includes oracle mock and first-trade waiver handling

2. **PnL Calculation End-to-End**: Tests PnL calculation with realistic parameters
   - Verifies invariants hold through the full PnL calculation flow
   - Tests with realistic entry and exit prices

3. **Liquidity Pool Round-Trip End-to-End**: Tests repeated deposit/withdraw cycles with realistic parameters
   - Verifies conservation and minimum-share invariants hold across many operations
   - Includes tiny-amount and maximum-integer boundary cases

## Adding New Invariants

When adding new arithmetic-heavy entrypoints to the contracts:

1. **Identify the arithmetic operations**: Determine what calculations are performed
2. **Define invariants**: Specify what properties must always hold
3. **Add property tests**: Create proptest tests for each invariant
4. **Constrain inputs**: Ensure generated inputs are realistic and valid
5. **Add integration tests**: Verify invariants hold end-to-end
6. **Update this document**: Document the new invariants

### Example: Adding a New Fee Component

If you add a new fee component (e.g., "protocol fee"), you should:

1. Add a property test for the calculation:
```rust
#[test]
fn prop_protocol_fee_no_overflow(
    fee_amount in 1_i128..=MAX_TRADE_AMOUNT,
    protocol_rate_bps in 0u32..=MAX_PROTOCOL_RATE_BPS,
) {
    let protocol_amount = fee_amount
        .checked_mul(protocol_rate_bps as i128)
        .and_then(|v| v.checked_div(10_000));

    prop_assert!(protocol_amount.is_some(), "protocol fee calculation should not overflow");
}
```

2. Update the fee components sum test to include the new component
3. Add an integration test to verify the new component in production
4. Update this document with the new invariant

## Regression Testing

If a property test fails:

1. **Save the failing seed**: Proptest will output a seed that reproduces the failure
2. **Reproduce the failure**: Run with the specific seed:
   ```bash
   cargo test --test test_arithmetic_invariants prop_fee_calculation_no_overflow -- --exact
   ```
3. **Debug the issue**: Use the seed to reproduce and debug the failure
4. **Fix the underlying code**: Address the root cause in the contract logic
5. **Add a regression test**: Add a unit test with the specific failing input
6. **Update proptest regressions**: If needed, update the proptest regression file

## CI Integration

The arithmetic invariant tests run as part of the standard CI pipeline. Any failure blocks the merge, ensuring rounding drift and conservation violations are caught before they reach production.
