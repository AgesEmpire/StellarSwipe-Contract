# Asset-Pair Validation (Issue #992)

Centralized asset-pair validation for `auto_trade` and `trade_executor`. Before any offer, swap, or token operation is attempted, a pair must pass three checks:

1. **Registered** — both assets are present in the configured asset registry.
2. **Distinct** — the two assets differ; an identical pair is rejected.
3. **Route-supported** — the selected route quotes the exact requested direction.

## Where validation lives

Shared logic lives in `stellar_swipe_common::pair_validation`:

| Function | Check |
|----------|-------|
| `validate_distinct` | Local check that `base != quote` — no external calls |
| `validate_registered_distinct_pair` | Registry lookups for both assets + distinct check |
| `route_supports_pair` | `get_best_ask(from, to)` on the route, requires positive price and quantity |
| `validate_pair_for_route` | Full gate: registered + distinct + route-supported |

Contract wiring:

- `auto_trade::amm_bridge::validate_signal_pair` — gates `execute_trade` and the AMM-bridge swap path.
- `trade_executor` — gates `swap` (and the copy-trade settlement path) before offers are placed.

## Enforcement model

Validation is enforced **when an asset registry is configured** on the trading contract. Contracts that have never had a registry configured (legacy deployments, u32-asset-id signal paths) keep their previous behavior. Admins enable enforcement by calling `set_asset_registry`.

## Supported-pair source and update authority

| Source | What it decides | Update authority |
|--------|-----------------|------------------|
| **Asset registry** (`shared::asset_registry::AssetRegistryContract`) | Which assets may be traded at all | Admin-only via `register_asset` / `update_asset` on the registry |
| **Route** (SDEX router in `trade_executor`, enabled AMM sources in `auto_trade`) | Which pairs are executable and in which direction | Admin-only via `set_sdex_router` / `register_amm_source` |

Both contracts point at the same registry contract address, configured admin-only via their `set_asset_registry` entrypoints.

Because the registry and the route are consulted on **every** validation:

- A **registry change** (asset deregistered, new asset added) takes effect on the very next trade.
- A **stale route configuration** (route no longer deployed, ABI changed, or no longer quoting the pair) rejects the trade with `RouteUnsupported` instead of failing mid-swap.

## Pair-direction semantics

- **Identical pair** (`base == quote`) — always rejected with `IdenticalAssets`, before any external call.
- **Reversed pair** — validated symmetrically: both assets must be registered (order-independent) and distinct, then the route is queried for the exact requested direction. A reversed pair is accepted when the route quotes it and rejected with `RouteUnsupported` when it does not.

## Failure modes

| Condition | Error |
|-----------|-------|
| No asset registry configured | `RegistryNotConfigured` |
| Base asset not in registry | `BaseAssetNotRegistered` |
| Quote asset not in registry | `QuoteAssetNotRegistered` |
| `base == quote` | `IdenticalAssets` |
| No route configured | `RouteNotConfigured` |
| Route does not quote the pair direction | `RouteUnsupported` |

External calls fail **closed**: a failed cross-contract call (wrong address, missing entrypoint) is treated as "not registered" / "route does not support the pair".

## Test coverage

- Common crate (`pair_validation`): identical pair, unregistered base/quote, reversed pair symmetry, registry change rejecting a previously valid pair, stale route configuration, missing registry/route config, full gate accept path.
- `auto_trade` and `trade_executor`: entrypoint-level tests wiring the gates into `execute_trade` and `swap`.
