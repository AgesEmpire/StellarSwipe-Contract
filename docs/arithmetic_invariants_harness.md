# Arithmetic Invariants Proptest Harness

## Overview

This document describes the property-based testing harness for fee and PnL arithmetic invariants in the StellarSwipe contracts. The harness uses `proptest` to generate randomized valid input ranges and asserts core arithmetic invariants hold for fee-splitting calculations in `fee_collector` and realized-PnL calculations in `trade_executor`.

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

The test harness is integrated into the CI test suite via the `[[test]]` configuration in `integration_tests/Cargo.toml`:

```toml
[[test]]
name = "test_arithmetic_invariants"
path = "tests/integration/test_arithmetic_invariants.rs"
```

This ensures the tests run automatically on every CI build.

## Performance Considerations

- **Test cases**: 1,000 cases per property (configurable)
- **Runtime**: Approximately 1-2 minutes for the full suite
- **Memory**: Minimal (no large allocations)
- **CI budget**: Designed to run within reasonable CI time limits

If CI time becomes a concern, you can:
1. Reduce the number of test cases in `ProptestConfig`
2. Run specific property tests instead of the full suite
3. Use proptest's `fork` mode to parallelize tests

## References

- [Proptest Documentation](https://altsysrq.github.io/proptest-book/)
- [Fee Collector Contract](../stellar-swipe/contracts/fee_collector/)
- [Trade Executor Contract](../stellar-swipe/contracts/trade_executor/)
- [Chaos Test Documentation](./chaos_test.md)
