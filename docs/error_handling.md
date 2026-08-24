# Error Handling and Recovery Patterns

This document describes the standardized error handling strategy used across Stellar Swipe contracts.

## Error categories

- `Validation`: Input validation, malformed requests, invalid amounts.
- `Authorization`: Missing or incorrect auth, admin-only operations.
- `ExternalDependency`: Oracle failures, cross-contract invocation failures.
- `Arithmetic`: Overflow / division by zero / invalid math.
- `Upgrade`: Upgrade and migration state issues.
- `Network`: Real-time network condition or congestion pricing issues.
- `Recovery`: Errors that require retry or manual intervention.

## Recovery mechanisms

- Contract-level error reporting stores the latest error event and recovery recommendation.
- Failed fee collection operations can be queued for retry via an on-chain recovery queue.
- Automatic retries are exposed through dedicated retry helper methods.
- Error reports include a recovery strategy and timestamp for auditability.

## Documentation

Developers should use these categories when mapping contract errors to frontend alerts or off-chain monitoring systems.
