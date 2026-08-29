# Error Taxonomy

Every `#[contracterror]` enum in the workspace follows a consistent taxonomy so
that auditors, integrators, and monitoring systems can classify errors without
reading contract source code.

## Categories

These categories are defined in `contracts/shared/src/errors.rs` as the
`ErrorCategory` enum. Each contract error maps to exactly one category.

| Category | Code | Meaning |
|---|---|---|
| `Validation` | 1 | Bad input: malformed arguments, out-of-range values, invalid state transitions |
| `Authorization` | 2 | Missing or incorrect auth: admin-only, allowlist, multisig |
| `ExternalDependency` | 3 | Oracle failure, cross-contract call failure, external adapter error |
| `Arithmetic` | 4 | Overflow, underflow, division by zero, precision loss |
| `Upgrade` | 5 | Incompatible contract version, storage layout mismatch |
| `Network` | 6 | Congestion pricing, real-time condition invalid |
| `Recovery` | 7 | Retry/defer/escalate: failed queue, retry limit, timelock not elapsed |

## Recovery strategies

Defined in `contracts/shared/src/errors.rs` as `RecoveryStrategy`.

| Strategy | Code | When to use |
|---|---|---|
| `Retry` | 1 | Transient failure; caller may re-submit the same transaction |
| `Defer` | 2 | Depends on time/ledger; retry after the relevant timelock elapses |
| `Escalate` | 3 | Requires elevated privileges (admin, multisig) to resolve |
| `ManualReview` | 4 | Cannot be resolved programmatically; human intervention required |

## Rules for adding new errors

1. **Pick a category** from the table above. If no existing category fits,
   open a PR to extend `ErrorCategory` before adding the error.
2. **Never reuse a numeric code.** Use the next unused integer in the enum. If
   an old variant is deprecated, add it to the `"deprecated"` list in the
   baseline JSON — never recycle its number.
3. **Never renumber an existing variant.** Clients and monitoring systems
   depend on stable numeric values. The `check_error_codes.py` CI step will
   fail if a renumbering is detected.
4. **Add a baseline entry.** After adding the new variant, run
   `python3 stellar-swipe/scripts/check_error_codes.py`; it will auto-update
   the relevant `error-baselines/<crate>.json` file. Commit the updated
   baseline alongside the new code.
5. **Document the recovery strategy** in the contract's public API doc comment
   so front-ends and integrators know what action to recommend to users.

## Per-contract error inventory

The baseline JSON files in `stellar-swipe/error-baselines/` are the canonical
source of truth. The table below is a human-readable summary; the JSON files
are authoritative for CI.

### `fee_collector` — `ContractError`

| Code | Variant | Category | Recovery |
|---|---|---|---|
| 1 | `AlreadyInitialized` | Validation | — |
| 2 | `NotInitialized` | Validation | Escalate |
| 3 | `Unauthorized` | Authorization | Escalate |
| 4 | `InvalidAmount` | Validation | — |
| 5 | `InsufficientTreasuryBalance` | Recovery | Defer |
| 6 | `WithdrawalNotQueued` | Validation | — |
| 7 | `TimelockNotElapsed` | Recovery | Defer |
| 8 | `ArithmeticOverflow` | Arithmetic | — |
| 9 | `FeeRateTooHigh` | Validation | Escalate |
| 10 | `FeeRateTooLow` | Validation | Escalate |
| 11 | `OracleNotConfigured` | ExternalDependency | Escalate |
| 12 | `OracleConversionFailed` | ExternalDependency | Retry |
| 13 | `FeeRoundedToZero` | Arithmetic | — |
| 14 | `BurnRateTooHigh` | Validation | Escalate |
| 15 | `DivisionByZero` | Arithmetic | — |
| 16 | `InvalidFeeConfiguration` | Validation | Escalate |
| 17 | `NetworkConditionInvalid` | Network | Retry |
| 18 | `FailedCollectionNotFound` | Recovery | — |
| 19 | `RetryLimitExceeded` | Recovery | ManualReview |
| 20 | `IterationLimitExceeded` | Recovery | ManualReview |
| 21 | `WaterfallNotConfigured` | Validation | Escalate |
| 22 | `PreferredTokenInsufficient` | Recovery | Defer |
| 23 | `PayoutCurrencyUnchanged` | Validation | — |
| 24 | `InvalidMultiplierBounds` | Validation | — |
| 25 | `SelfReferralNotAllowed` | Validation | — |
| 26 | `ReferralAlreadyRegistered` | Validation | — |
| 27 | `IncompatibleContractVersion` | Upgrade | Escalate |
| 28 | `UnauthorizedCaller` | Authorization | Escalate |

### `stake_vault` — `StakeVaultError`

| Code | Variant | Category | Recovery |
|---|---|---|---|
| 1 | `NotInitialized` | Validation | Escalate |
| 2 | `Unauthorized` | Authorization | Escalate |
| 3 | `NoStake` | Validation | — |
| 4 | `StakeLocked` | Recovery | Defer |
| 5 | `ReentrancyDetected` | Authorization | — |
| 6 | `StakeBelowMinimum` | Validation | — |
| 7 | `ContractPaused` | Network | Defer |
| 8 | `TimelockRequired` | Recovery | Defer |
| 9 | `TimelockNotElapsed` | Recovery | Defer |
| 10 | `FlashLoanDetected` | Authorization | — |
| 11 | `InvalidSlashTier` | Validation | — |
| 12 | `StakeDurationNotElapsed` | Recovery | Defer |
| 13 | `NoDelegatedStake` | Validation | — |
| 14 | `UnstakeAlreadyQueued` | Validation | — |
| 15 | `NoUnstakeQueued` | Validation | — |
| 16 | `QueueEmpty` | Validation | — |
| 17 | `SlashNotFound` | Validation | — |
| 18 | `AppealWindowClosed` | Validation | — |
| 19 | `AppealAlreadyExists` | Validation | — |
| 20 | `AppealAlreadyResolved` | Validation | — |
| 21 | `InvalidAppealWindow` | Validation | — |
| 22 | `RateLimitExceeded` | Recovery | Defer |
| 23 | `InvalidAmount` | Validation | — |
| 24 | `RemainingStakeBelowMinimum` | Validation | — |
| 34 | `IncompatibleContractVersion` | Upgrade | Escalate |

### `auto_trade` — `AutoTradeError`

| Code | Variant | Category | Recovery |
|---|---|---|---|
| 1 | `InvalidAmount` | Validation | — |
| 2 | `Unauthorized` | Authorization | Escalate |
| 3 | `SignalNotFound` | Validation | — |
| 4 | `SignalExpired` | Validation | — |
| 5 | `InsufficientBalance` | Recovery | Defer |
| 6 | `InsufficientLiquidity` | ExternalDependency | Retry |
| 7 | `DailyTradeLimitExceeded` | Recovery | Defer |
| 8 | `PositionLimitExceeded` | Validation | — |
| 9 | `StopLossTriggered` | Validation | — |
| 10 | `StrategyNotFound` | Validation | — |
| 11 | `PositionAlreadyExists` | Validation | — |
| 12 | `InsufficientPriceHistory` | ExternalDependency | Defer |
| 13 | `RankingDisabled` | Validation | Escalate |
| 14 | `RateLimited` | Recovery | Defer |
| 15 | `PrivacyModeEnabled` | Authorization | — |
| 16 | `TradingPaused` | Network | Defer |
| 17–23 | portfolio/stat-arb | Validation | — |
| 24–27 | exit/insurance | Validation | — |
| 28 | `ReferralError` | Validation | — |
| 29 | `TWAPError` | Validation | — |
| 30–31 | correlation | Validation | — |
| 32–33 | conditional orders | Validation | — |
| 34 | `RateLimitExceeded` | Recovery | Defer |
| 35–39 | pairs trading | Validation | — |
| 40 | `OracleUnavailable` | ExternalDependency | Retry |
| 41 | `DcaError` | Validation | — |
| 42 | `MrStrategyError` | Validation | — |
| 43 | `AdminTransferError` | Authorization | Escalate |
| 44 | `RoutingPlanNotFound` | Validation | — |
| 45–46 | arbitrage | Validation | — |
| 47 | `SystemError` | Recovery | ManualReview |
| 48 | `SlippageExceeded` | Validation | Retry |
| 49 | `LastOracleForPair` | Validation | Escalate |
| 50 | `NotPaused` | Validation | — |

### `shared` — cross-contract errors

| Enum | Variant | Code | Category |
|---|---|---|---|
| `CrossContractError` | `UnauthorizedSigner` | 1 | Authorization |
| `CrossContractError` | `UnauthorizedCaller` | 2 | Authorization |
| `CrossContractError` | `InvalidPayload` | 3 | Validation |
| `CrossContractError` | `InvalidMessage` | 4 | Validation |
| `CrossContractError` | `MessageNotFound` | 5 | Validation |
| `CrossContractError` | `VersionMismatch` | 6 | Upgrade |
| `CrossContractError` | `CallDepthExceeded` | 7 | Recovery |
| `CrossContractError` | `ContractHashMismatch` | 8 | Upgrade |
| `CrossContractError` | `AlreadyDelivered` | 9 | Validation |
| `CrossContractError` | `CallerNotRegistered` | 10 | Authorization |
| `LiquidityPoolError` | `BelowMinimumLiquidity` | 1 | Validation |
| `VersionError` | `IncompatibleContractVersion` | 1 | Upgrade |
| `WasmHashError` | `UnexpectedContractVersion` | 1 | Upgrade |

## Deprecation process

Never reuse a numeric code. To retire an error variant:

1. Add the variant name and code to a `"deprecated"` list in the relevant
   `error-baselines/<crate>.json`:
   ```json
   "deprecated": [
     {"variant": "OldVariantName", "code": 99, "reason": "replaced by NewVariant (100)"}
   ]
   ```
2. Leave the Rust variant in the source with a `#[deprecated]` attribute and a
   doc comment explaining the replacement.
3. Commit the updated baseline and source together.

## Stability guarantee

All variant-to-code mappings are considered a public API. Once a code is
assigned and merged to `main`, the mapping is **permanent**. The
`check_error_codes.py` CI gate enforces this automatically.

See `docs/source-verification.md` for the full verification process.
