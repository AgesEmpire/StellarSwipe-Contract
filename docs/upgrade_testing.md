# Contract Upgrade Testing Framework

This document outlines the upgrade testing framework used by Stellar Swipe integration tests.

## Framework design

- The test harness simulates contract upgrades by re-registering a new implementation at the same contract address.
- Persistent and instance storage remains unchanged when a new WASM is registered.
- Tests validate that state is preserved, new functions are available, and admin controls remain enforced.

## Coverage

- Prior state is preserved for signals, positions, auth records, and closed positions.
- New v2-only API surface is available after upgrade.
- Upgrade-only functionality remains admin-restricted.
- Rollback simulations verify that state remains consistent if the contract implementation is re-registered.

## How to add new tests

1. Add a new integration test in `stellar-swipe/contracts/integration_tests/tests/integration/`.
2. Use `env.register_at(&cid, ContractV2, ())` to simulate the upgrade.
3. Assert both backward compatibility and new behavior after upgrade.
