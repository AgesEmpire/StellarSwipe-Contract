# Contract error taxonomy (Issue #1033)

Every contract surfaces failures as a `#[contracterror] enum ContractError`
(or a domain-specific error enum) with **stable numeric discriminants** —
frozen by `scripts/check_error_codes.py` and the baselines in
`error-baselines/`. Numbers are never renumbered or reused.

To let frontend clients and off-chain scripts respond to the *kind* of
failure without hard-coding per-contract numbers, each error maps to exactly
one category in `shared::errors::ErrorCategory`:

| Category (`slug`)              | Code | Meaning                                            | `is_transient` | Default strategy |
|--------------------------------|-----:|---------------------------------------------------|:--------------:|------------------|
| `validation`                   | 1    | Malformed / out-of-range input                     | no             | Escalate         |
| `authorization`                | 2    | Caller lacks permission                            | no             | Escalate         |
| `external_dependency`          | 3    | Oracle / token / cross-contract dependency failed  | yes            | Retry            |
| `arithmetic`                   | 4    | Overflow or division by zero                       | no             | Escalate         |
| `upgrade`                      | 5    | Version / migration mismatch                       | no             | Escalate         |
| `network`                      | 6    | Transient transport / gateway issue                | yes            | Retry            |
| `recovery`                     | 7    | Guardian / recovery-flow failure                   | no             | ManualReview     |
| `capacity_limit`               | 8    | Quota, rate limit, or batch/size cap reached       | yes            | Defer            |
| `invariant_violation`          | 9    | Operation would break a protocol invariant         | no             | ManualReview     |

`ErrorCategory::slug()`, `::is_transient()`, and `::default_strategy()`
return these values.

## Representative mapping per contract action

The four classifications integrators most need to distinguish are
**invalid input**, **unauthorized action**, **capacity limit**, and
**invariant break**. Representative error codes:

### fee_collector (`ContractError`)

| Action                        | Failure                              | Code | Category             |
|-------------------------------|--------------------------------------|-----:|----------------------|
| `set_fee_rate`                | rate above `MAX_FEE_RATE_BPS`         | 9    | validation           |
| `set_fee_rate`                | caller is not the admin              | 3    | authorization        |
| `set_fee_split_policy`        | `protocol_bps + provider_bps != 10000`| 16   | validation           |
| `queue_withdrawal` / claims   | treasury balance too low            | 5    | invariant_violation  |
| any write while paused        | contract paused                     | 34   | capacity_limit       |
| fee math                      | arithmetic overflow                 | 8    | arithmetic           |

### signal_registry (`Error`)

| Action              | Failure                          | Category            |
|---------------------|----------------------------------|---------------------|
| `submit_signal`     | empty / oversized metadata       | validation          |
| `submit_signal`     | daily submission cap reached     | capacity_limit      |
| admin-only setters  | caller not admin / not allowlisted | authorization     |
| reward accounting   | payout would exceed ledger total | invariant_violation |

### auto_trade / trade_executor (`ContractError`)

| Action            | Failure                            | Category            |
|-------------------|------------------------------------|---------------------|
| `execute_trade`   | amount ≤ 0 / bad asset pair        | validation          |
| `execute_trade`   | per-window trade cap hit           | capacity_limit      |
| keeper entrypoints| caller not an authorized keeper    | authorization       |
| settlement        | slippage beyond bound / stale oracle | external_dependency |

## Shared helpers

`common::join_rate_limit::JoinRateLimitError` and
`common::collateral_oracle::CollateralError` are domain error enums with
`#[repr(u32)]` discriminants that follow the same rules:

| Enum / variant                              | Code | Category             |
|---------------------------------------------|-----:|----------------------|
| `JoinRateLimitError::Exceeded`              | 1    | capacity_limit       |
| `JoinRateLimitError::Disabled`              | 2    | capacity_limit       |
| `CollateralError::PriceStale`               | 1    | external_dependency  |
| `CollateralError::PriceUnavailable`         | 2    | external_dependency  |
| `CollateralError::InvalidPrice`             | 3    | validation           |
| `CollateralError::Arithmetic`               | 4    | arithmetic           |

Integration coverage lives in
`contracts/fee_collector/src/tests/error_taxonomy_tests.rs`, which drives real
contract failures through the `fee_collector` client and asserts both the
numeric code and its taxonomy category.
