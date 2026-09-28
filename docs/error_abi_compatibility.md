# Error & ABI Compatibility

This document records the response shapes and failure behavior that the
contracts in this repository expect from **supported Stellar token
contracts** (SEP-41 style fungible tokens). It exists so that integrators
and reviewers can see, in one place, what we assume a token will return and
how we behave when it does not.

## Supported token response shapes

All token interactions go through the shared token client. The following
return shapes are considered **supported** and are the only ones the
contracts are written against.

### `transfer(from, to, amount)`

| Outcome | Expected return | Notes |
| --- | --- | --- |
| Success | `Ok(())` (unit / `Void`) | No value is consumed. |
| Failure | `Err(TokenError)` | Any contract error is propagated as a stable error. |

Assumptions:

- The token is a standard SEP-41 fungible token: `transfer` returns unit on
  success and a typed error on failure.
- The token does **not** take a fee on transfer. A token that credits less
  than `amount` to `to` is treated as unsupported (see below).
- The token does not re-enter the calling contract during `transfer`.

### `balance(id)`

| Outcome | Expected return | Notes |
| --- | --- | --- |
| Success | `i128` | Non-negative balance for the queried account. |
| Failure | `Err(TokenError)` | Propagated as a stable error. |

### `decimals()`

| Outcome | Expected return | Notes |
| --- | --- | --- |
| Success | `u32` | Number of decimal places. |
| Failure | `Err(TokenError)` | Propagated as a stable error. |

## Fail-closed behavior

Unexpected token responses must **fail closed**: the calling contract must
revert rather than silently continue with an unknown state. The following
cases are treated as unsupported and produce a stable, documented error:

1. **Malformed / unexpected return value** — a `transfer` that returns a
   non-unit value, or a `balance`/`decimals` that returns a value of the
   wrong type, is rejected.
2. **Contract error** — any `Err` returned by the token is surfaced as an
   error; it is never swallowed.
3. **Fee-on-transfer** — if the recipient balance does not increase by
   exactly `amount`, the transfer is rejected as unsupported.

These checks are intentionally strict: a token that does not match the
supported shapes above is not a supported Stellar asset for this contract.

## Test coverage

The compatibility tests live alongside the token client and cover:

- **Valid returns** — `transfer` returning unit, `balance` returning `i128`,
  `decimals` returning `u32`.
- **Contract errors** — the token returning `Err(TokenError)`; the error is
  propagated and the call reverts.
- **Malformed response fixtures** — mock tokens that return unexpected
  values (e.g. a non-unit `transfer` result, a fee-on-transfer balance
  delta); the call fails closed with the stable error.

When adding a new supported token shape, update this document and add a
matching fixture to the compatibility tests.
