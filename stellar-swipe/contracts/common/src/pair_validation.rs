//! Centralized asset-pair validation for `auto_trade` and `trade_executor` (Issue #992).
//!
//! Before this module, each contract accepted any pair of token addresses and
//! let unsupported pairs fail at swap/offer time — after allowances were
//! granted or state was already mutated. This module centralizes the checks a
//! pair must pass *before* any external call or state mutation:
//!
//! 1. **Registered** — both assets must be present in the configured asset
//!    registry (`shared::asset_registry::AssetRegistryContract`).
//! 2. **Distinct** — the two assets must differ; an identical pair is rejected.
//! 3. **Route-supported** — the selected route (SDEX router in
//!    `trade_executor`, enabled AMM sources in `auto_trade`) must quote the
//!    exact requested direction via its `get_best_ask(from, to)` entrypoint.
//!
//! # Supported-pair source and update authority
//!
//! - The **asset registry** is the single source of truth for which assets may
//!   be traded. It is updated **admin-only** through
//!   [`shared::asset_registry::AssetRegistryContract::register_asset`] and
//!   `update_asset`. Both `auto_trade` and `trade_executor` point at the same
//!   registry contract address, configured admin-only via their
//!   `set_asset_registry` entrypoints. Because the registry is consulted on
//!   every validation, a registry change (asset deregistered or a new one
//!   added) takes effect on the very next trade.
//! - The **route** (SDEX router in `trade_executor`, AMM sources in
//!   `auto_trade`) is the source of truth for which pairs are executable. It
//!   is updated admin-only (`set_sdex_router` / `register_amm_source`).
//!   Validation queries the route's live `get_best_ask` output on every trade,
//!   so a stale route configuration (a route that no longer quotes the pair)
//!   rejects the trade instead of failing mid-swap.
//!
//! See [`docs/asset_pair_validation.md`](../../../docs/asset_pair_validation.md)
//! for the full design notes.
//!
//! # Enforcement model
//!
//! Validation is enforced when an asset registry is configured on the trading
//! contract. Contracts that have never had a registry configured (legacy
//! deployments, u32-asset-id signal paths) keep their previous behavior;
//! admins enable enforcement simply by calling `set_asset_registry`.
//!
//! # Pair-direction semantics
//!
//! - **Identical pair** (`base == quote`) — always rejected with
//!   [`PairValidationError::IdenticalAssets`], before any external call.
//! - **Reversed pair** — validated symmetrically: both assets must be
//!   registered (order-independent) and distinct, then the route is queried
//!   for the exact requested direction. A reversed pair is accepted when the
//!   route quotes it and rejected with [`PairValidationError::RouteUnsupported`]
//!   when the route does not.

use soroban_sdk::{contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

use shared::asset_registry::AssetMetadata;

/// Entrypoint name on the asset registry
/// (`get_asset_metadata(asset) -> Option<AssetMetadata>`).
pub const FN_GET_ASSET_METADATA: &str = "get_asset_metadata";

/// Route entrypoint queried to confirm a route supports a pair
/// (`get_best_ask(from, to) -> (price, qty)`).
pub const FN_GET_BEST_ASK: &str = "get_best_ask";

/// Reason an asset pair was rejected before execution (Issue #992).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PairValidationError {
    /// No asset registry is configured on the contract.
    RegistryNotConfigured = 1,
    /// The base asset is not registered in the asset registry.
    BaseAssetNotRegistered = 2,
    /// The quote asset is not registered in the asset registry.
    QuoteAssetNotRegistered = 3,
    /// The two assets are identical; distinct assets are required.
    IdenticalAssets = 4,
    /// No route is configured to execute the pair.
    RouteNotConfigured = 5,
    /// The selected route does not support the requested pair direction.
    RouteUnsupported = 6,
}

/// True when `asset` is present in the asset registry at `registry`.
///
/// A failed cross-contract call (wrong address, missing entrypoint) is treated
/// as "not registered" — the check fails closed.
pub fn asset_is_registered(env: &Env, registry: &Address, asset: &Address) -> bool {
    let sym = Symbol::new(env, FN_GET_ASSET_METADATA);
    let mut args = Vec::<Val>::new(env);
    args.push_back(asset.clone().into_val(env));
    matches!(
        env.try_invoke_contract::<Option<AssetMetadata>, soroban_sdk::Error>(registry, &sym, args),
        Ok(Ok(Some(_)))
    )
}

/// Check that `base` and `quote` are distinct. Purely local — no external calls.
pub fn validate_distinct(base: &Address, quote: &Address) -> Result<(), PairValidationError> {
    if base == quote {
        return Err(PairValidationError::IdenticalAssets);
    }
    Ok(())
}

/// Check that both assets are registered in `registry` and that they differ.
///
/// The registration check is symmetric: a reversed pair passes as long as both
/// assets are registered.
pub fn validate_registered_distinct_pair(
    env: &Env,
    registry: &Address,
    base: &Address,
    quote: &Address,
) -> Result<(), PairValidationError> {
    validate_distinct(base, quote)?;
    if !asset_is_registered(env, registry, base) {
        return Err(PairValidationError::BaseAssetNotRegistered);
    }
    if !asset_is_registered(env, registry, quote) {
        return Err(PairValidationError::QuoteAssetNotRegistered);
    }
    Ok(())
}

/// Confirm `route` quotes the pair in the requested direction via
/// `get_best_ask(from, to) -> (price, qty)`.
///
/// A route "supports" a pair when the call succeeds and returns a positive
/// price and quantity. A failed call (route no longer deployed / ABI changed)
/// or a `(0, 0)` quote (route no longer lists the pair) both mean the route
/// does not support the pair — this is what makes stale route configuration
/// fail closed instead of failing mid-swap.
pub fn route_supports_pair(
    env: &Env,
    route: &Address,
    from: &Address,
    to: &Address,
) -> Result<(), PairValidationError> {
    let sym = Symbol::new(env, FN_GET_BEST_ASK);
    let mut args = Vec::<Val>::new(env);
    args.push_back(from.clone().into_val(env));
    args.push_back(to.clone().into_val(env));
    let supported =
        match env.try_invoke_contract::<(i128, i128), soroban_sdk::Error>(route, &sym, args) {
            Ok(Ok((price, qty))) => price > 0 && qty > 0,
            _ => false,
        };
    if supported {
        Ok(())
    } else {
        Err(PairValidationError::RouteUnsupported)
    }
}

/// Full pre-execution pair gate: registered + distinct + route-supported.
///
/// `registry`/`route` are `None` when the calling contract has no registry or
/// route configured, in which case the corresponding configuration error is
/// returned. Callers that want opt-in enforcement should only invoke this when
/// a registry is configured (see module docs for the enforcement model).
pub fn validate_pair_for_route(
    env: &Env,
    registry: Option<&Address>,
    route: Option<&Address>,
    base: &Address,
    quote: &Address,
) -> Result<(), PairValidationError> {
    let registry = registry.ok_or(PairValidationError::RegistryNotConfigured)?;
    validate_registered_distinct_pair(env, registry, base, quote)?;
    let route = route.ok_or(PairValidationError::RouteNotConfigured)?;
    route_supports_pair(env, route, base, quote)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Env, String as SdkString};

    use shared::asset_registry::AssetRegistryContract;

    // ── Mock route ───────────────────────────────────────────────────────────

    #[contract]
    struct MockRoute;

    #[contractimpl]
    impl MockRoute {
        pub fn get_best_ask(env: Env, _from: Address, _to: Address) -> (i128, i128) {
            env.storage()
                .instance()
                .get(&Symbol::new(&env, "ask"))
                .unwrap_or((0, 0))
        }

        pub fn set_best_ask(env: Env, price: i128, qty: i128) {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "ask"), &(price, qty));
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_registry_with_assets(env: &Env, assets: &[&Address]) -> Address {
        let admin = Address::generate(env);
        let registry_id = env.register(AssetRegistryContract, ());
        let client = shared::asset_registry::AssetRegistryContractClient::new(env, &registry_id);
        client.initialize(&admin);
        for asset in assets {
            client.register_asset(
                &admin,
                asset,
                &SdkString::from_str(env, "TOK"),
                &7u32,
                &None,
            );
        }
        registry_id
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    #[test]
    fn identical_pair_rejected_before_any_external_call() {
        let env = Env::default();
        env.mock_all_auths();
        let asset = Address::generate(&env);
        let registry = make_registry_with_assets(&env, &[&asset]);

        assert_eq!(
            validate_registered_distinct_pair(&env, &registry, &asset, &asset),
            Err(PairValidationError::IdenticalAssets)
        );
    }

    #[test]
    fn unregistered_base_asset_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let registry = make_registry_with_assets(&env, &[&quote]);

        assert_eq!(
            validate_registered_distinct_pair(&env, &registry, &base, &quote),
            Err(PairValidationError::BaseAssetNotRegistered)
        );
    }

    #[test]
    fn unregistered_quote_asset_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let registry = make_registry_with_assets(&env, &[&base]);

        assert_eq!(
            validate_registered_distinct_pair(&env, &registry, &base, &quote),
            Err(PairValidationError::QuoteAssetNotRegistered)
        );
    }

    #[test]
    fn registered_distinct_pair_passes() {
        let env = Env::default();
        env.mock_all_auths();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let registry = make_registry_with_assets(&env, &[&base, &quote]);

        assert!(validate_registered_distinct_pair(&env, &registry, &base, &quote).is_ok());
    }

    #[test]
    fn reversed_pair_passes_registration_symmetrically() {
        let env = Env::default();
        env.mock_all_auths();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let registry = make_registry_with_assets(&env, &[&base, &quote]);

        assert!(validate_registered_distinct_pair(&env, &registry, &quote, &base).is_ok());
    }

    /// A registry that loses an asset (change) rejects the pair on the next check.
    #[test]
    fn registry_change_rejects_previously_valid_pair() {
        let env = Env::default();
        env.mock_all_auths();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);

        // Registry A has both assets — valid.
        let registry_a = make_registry_with_assets(&env, &[&base, &quote]);
        assert!(validate_registered_distinct_pair(&env, &registry_a, &base, &quote).is_ok());

        // Registry B only has the base (quote was never added / deregistered).
        let registry_b = make_registry_with_assets(&env, &[&base]);
        assert_eq!(
            validate_registered_distinct_pair(&env, &registry_b, &base, &quote),
            Err(PairValidationError::QuoteAssetNotRegistered)
        );
    }

    #[test]
    fn route_that_quotes_pair_is_supported() {
        let env = Env::default();
        env.mock_all_auths();
        let route = env.register(MockRoute, ());
        MockRouteClient::new(&env, &route).set_best_ask(&100i128, &1_000_000i128);
        let from = Address::generate(&env);
        let to = Address::generate(&env);

        assert!(route_supports_pair(&env, &route, &from, &to).is_ok());
    }

    #[test]
    fn route_that_does_not_quote_pair_is_unsupported() {
        let env = Env::default();
        env.mock_all_auths();
        let route = env.register(MockRoute, ());
        MockRouteClient::new(&env, &route).set_best_ask(&0i128, &0i128);
        let from = Address::generate(&env);
        let to = Address::generate(&env);

        assert_eq!(
            route_supports_pair(&env, &route, &from, &to),
            Err(PairValidationError::RouteUnsupported)
        );
    }

    /// Stale route config: a route that previously quoted the pair stops doing so.
    #[test]
    fn stale_route_configuration_rejects_pair() {
        let env = Env::default();
        env.mock_all_auths();
        let route = env.register(MockRoute, ());
        let client = MockRouteClient::new(&env, &route);
        let from = Address::generate(&env);
        let to = Address::generate(&env);

        client.set_best_ask(&100i128, &1_000_000i128);
        assert!(route_supports_pair(&env, &route, &from, &to).is_ok());

        // Route stops quoting the pair (e.g. pair delisted from the router).
        client.set_best_ask(&0i128, &0i128);
        assert_eq!(
            route_supports_pair(&env, &route, &from, &to),
            Err(PairValidationError::RouteUnsupported)
        );
    }

    #[test]
    fn missing_route_configuration_reported() {
        let env = Env::default();
        env.mock_all_auths();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let registry = make_registry_with_assets(&env, &[&base, &quote]);

        assert_eq!(
            validate_pair_for_route(&env, Some(&registry), None, &base, &quote),
            Err(PairValidationError::RouteNotConfigured)
        );
    }

    #[test]
    fn missing_registry_configuration_reported() {
        let env = Env::default();
        env.mock_all_auths();
        let route = env.register(MockRoute, ());
        MockRouteClient::new(&env, &route).set_best_ask(&100i128, &1_000_000i128);
        let base = Address::generate(&env);
        let quote = Address::generate(&env);

        assert_eq!(
            validate_pair_for_route(&env, None, Some(&route), &base, &quote),
            Err(PairValidationError::RegistryNotConfigured)
        );
    }

    #[test]
    fn full_gate_accepts_registered_distinct_supported_pair() {
        let env = Env::default();
        env.mock_all_auths();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let registry = make_registry_with_assets(&env, &[&base, &quote]);
        let route = env.register(MockRoute, ());
        MockRouteClient::new(&env, &route).set_best_ask(&100i128, &1_000_000i128);

        assert!(
            validate_pair_for_route(&env, Some(&registry), Some(&route), &base, &quote,).is_ok()
        );

        // Reversed direction is accepted when the route quotes it too.
        assert!(
            validate_pair_for_route(&env, Some(&registry), Some(&route), &quote, &base,).is_ok()
        );
    }
}
