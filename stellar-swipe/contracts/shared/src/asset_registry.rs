//! Asset metadata registry contract (Issue #700).
//!
//! Provides a standalone registry storing symbol, decimals, and issuer address
//! per supported asset so that other contracts can look up this metadata from a
//! single source of truth instead of duplicating it.
//!
//! # Entrypoints
//! - `register_asset(admin, asset, symbol, decimals, issuer)` — admin-only, registers a new asset
//! - `update_asset(admin, asset, symbol, decimals, issuer)` — admin-only, updates existing metadata
//! - `get_asset_metadata(asset)` — read-only, returns metadata for a given asset contract address

use soroban_sdk::{contracterror, contracttype, Address, Env, String};

// The `#[contract]` implementation of the registry is intentionally gated
// behind `test` / the `testutils` feature so its exported WASM entrypoints
// (`initialize`, `register_asset`, …) are **not** linked into every contract
// that depends on the `shared` library crate. Because `shared` is a plain
// `lib`, an always-compiled `#[contractimpl]` would export those symbols into
// each downstream contract's WASM binary, colliding with the contract's own
// `initialize` at link time. The `AssetMetadata` type (and error enum) are kept
// available in production so call sites can deserialize registry responses
// cross-contract via `try_invoke_contract`.

// ── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AssetRegistryError {
    AlreadyRegistered = 1,
    NotRegistered = 2,
    Unauthorized = 3,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Full metadata for a single asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetMetadata {
    /// Short symbol / ticker (e.g. "USDC", "XLM").
    pub symbol: String,
    /// Number of decimal places (Stellar standard is 7).
    pub decimals: u32,
    /// Optional issuer address. `None` for native XLM.
    pub issuer: Option<Address>,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

#[cfg(any(test, feature = "testutils"))]
#[contracttype]
#[derive(Clone)]
#[allow(dead_code)]
enum AssetRegistryStorage {
    Admin,
    Metadata(Address),
}

// ── Contract ─────────────────────────────────────────────────────────────────

// The `contract`/`contractimpl` macros are only needed where the registry
// contract is compiled (tests/`testutils`); import them here, gated to match.
#[cfg(any(test, feature = "testutils"))]
use soroban_sdk::{contract, contractimpl};

#[cfg(any(test, feature = "testutils"))]
#[contract]
pub struct AssetRegistryContract;

#[cfg(any(test, feature = "testutils"))]
#[contractimpl]
impl AssetRegistryContract {
    // ── Initialization ────────────────────────────────────────────────────────

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&AssetRegistryStorage::Admin) {
            panic!("already initialized");
        }
        env.storage()
            .instance()
            .set(&AssetRegistryStorage::Admin, &admin);
    }

    fn require_admin(env: &Env) -> Result<Address, AssetRegistryError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&AssetRegistryStorage::Admin)
            .ok_or(AssetRegistryError::Unauthorized)?;
        admin.require_auth();
        Ok(admin)
    }

    // ── Entrypoints ───────────────────────────────────────────────────────────

    /// Register a new asset with its metadata. Admin-only.
    pub fn register_asset(
        env: Env,
        admin: Address,
        asset: Address,
        symbol: String,
        decimals: u32,
        issuer: Option<Address>,
    ) -> Result<AssetMetadata, AssetRegistryError> {
        let actual_admin = Self::require_admin(&env)?;
        if admin != actual_admin {
            return Err(AssetRegistryError::Unauthorized);
        }
        let key = AssetRegistryStorage::Metadata(asset.clone());
        if env.storage().instance().has(&key) {
            return Err(AssetRegistryError::AlreadyRegistered);
        }
        let meta = AssetMetadata {
            symbol,
            decimals,
            issuer,
        };
        env.storage().instance().set(&key, &meta);
        Ok(meta)
    }

    /// Update metadata for an already-registered asset. Admin-only.
    pub fn update_asset(
        env: Env,
        admin: Address,
        asset: Address,
        symbol: String,
        decimals: u32,
        issuer: Option<Address>,
    ) -> Result<AssetMetadata, AssetRegistryError> {
        let actual_admin = Self::require_admin(&env)?;
        if admin != actual_admin {
            return Err(AssetRegistryError::Unauthorized);
        }
        let key = AssetRegistryStorage::Metadata(asset.clone());
        if !env.storage().instance().has(&key) {
            return Err(AssetRegistryError::NotRegistered);
        }
        let meta = AssetMetadata {
            symbol,
            decimals,
            issuer,
        };
        env.storage().instance().set(&key, &meta);
        Ok(meta)
    }

    /// Read-only: look up metadata for a registered asset.
    pub fn get_asset_metadata(env: Env, asset: Address) -> Option<AssetMetadata> {
        let key = AssetRegistryStorage::Metadata(asset);
        env.storage().instance().get(&key)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn setup_env() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(AssetRegistryContract, ());
        let client = AssetRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin);
        (env, contract_id, admin)
    }

    #[test]
    fn test_initialize_sets_admin() {
        let (env, _contract_id, admin) = setup_env();
        let asset = Address::generate(&env);
        let client = AssetRegistryContractClient::new(&env, &_contract_id);
        let meta = client.register_asset(
            &admin,
            &asset,
            &String::from_str(&env, "USDC"),
            &7u32,
            &None,
        );
        assert_eq!(meta.symbol, String::from_str(&env, "USDC"));
    }

    #[test]
    fn test_register_and_lookup() {
        let (env, _contract_id, admin) = setup_env();
        let client = AssetRegistryContractClient::new(&env, &_contract_id);
        let asset = Address::generate(&env);
        let issuer = Address::generate(&env);

        let result = client.register_asset(
            &admin,
            &asset,
            &String::from_str(&env, "USDC"),
            &7u32,
            &Some(issuer.clone()),
        );
        assert_eq!(result.symbol, String::from_str(&env, "USDC"));

        let meta = client.get_asset_metadata(&asset).unwrap();
        assert_eq!(meta.symbol, String::from_str(&env, "USDC"));
        assert_eq!(meta.decimals, 7);
        assert_eq!(meta.issuer, Some(issuer));
    }

    #[test]
    fn test_register_duplicate_fails() {
        let (env, _contract_id, admin) = setup_env();
        let client = AssetRegistryContractClient::new(&env, &_contract_id);
        let asset = Address::generate(&env);

        client.register_asset(&admin, &asset, &String::from_str(&env, "XLM"), &7u32, &None);

        let result =
            client.try_register_asset(&admin, &asset, &String::from_str(&env, "XLM"), &7u32, &None);
        assert_eq!(result, Err(Ok(AssetRegistryError::AlreadyRegistered)));
    }

    #[test]
    fn test_update_asset() {
        let (env, _contract_id, admin) = setup_env();
        let client = AssetRegistryContractClient::new(&env, &_contract_id);
        let asset = Address::generate(&env);

        client.register_asset(
            &admin,
            &asset,
            &String::from_str(&env, "USDC"),
            &7u32,
            &None,
        );

        let issuer = Address::generate(&env);
        let result = client.update_asset(
            &admin,
            &asset,
            &String::from_str(&env, "USDC"),
            &6u32,
            &Some(issuer.clone()),
        );
        assert_eq!(result.decimals, 6);

        let meta = client.get_asset_metadata(&asset).unwrap();
        assert_eq!(meta.decimals, 6);
        assert_eq!(meta.issuer, Some(issuer));
    }

    #[test]
    fn test_update_unregistered_fails() {
        let (env, _contract_id, admin) = setup_env();
        let client = AssetRegistryContractClient::new(&env, &_contract_id);
        let asset = Address::generate(&env);

        let result = client.try_update_asset(
            &admin,
            &asset,
            &String::from_str(&env, "NONEXISTENT"),
            &7u32,
            &None,
        );
        assert_eq!(result, Err(Ok(AssetRegistryError::NotRegistered)));
    }

    #[test]
    fn test_lookup_unregistered_returns_none() {
        let (env, contract_id, admin) = setup_env();
        let client = AssetRegistryContractClient::new(&env, &contract_id);
        let _ = admin;

        let asset = Address::generate(&env);
        let meta = client.get_asset_metadata(&asset);
        assert!(meta.is_none());
    }

    #[test]
    fn test_register_xlm_native_asset() {
        let (env, _contract_id, admin) = setup_env();
        let client = AssetRegistryContractClient::new(&env, &_contract_id);
        let xlm_asset = Address::generate(&env);

        let result = client.register_asset(
            &admin,
            &xlm_asset,
            &String::from_str(&env, "XLM"),
            &7u32,
            &None,
        );
        assert_eq!(result.symbol, String::from_str(&env, "XLM"));

        let meta = client.get_asset_metadata(&xlm_asset).unwrap();
        assert_eq!(meta.symbol, String::from_str(&env, "XLM"));
        assert_eq!(meta.decimals, 7);
        assert!(meta.issuer.is_none());
    }

    #[test]
    fn test_unauthorized_register_fails() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(AssetRegistryContract, ());
        let client = AssetRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin);

        let impostor = Address::generate(&env);
        let asset = Address::generate(&env);
        let result = client.try_register_asset(
            &impostor,
            &asset,
            &String::from_str(&env, "FAKE"),
            &7u32,
            &None,
        );
        assert!(result.is_err());
    }

    /// Test: cross-contract integration — existing contract queries the registry.
    #[test]
    fn cross_contract_lookup_via_registry() {
        // Simulates how an existing contract (e.g. oracle, fee_collector) would
        // query the registry instead of hardcoding asset metadata.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry_id = env.register(AssetRegistryContract, ());
        let registry = AssetRegistryContractClient::new(&env, &registry_id);
        registry.initialize(&admin);

        // Register XLM (native) and USDC (with issuer) metadata.
        let xlm = Address::generate(&env);
        registry.register_asset(&admin, &xlm, &String::from_str(&env, "XLM"), &7u32, &None);

        let usdc_issuer = Address::generate(&env);
        let usdc = Address::generate(&env);
        registry.register_asset(
            &admin,
            &usdc,
            &String::from_str(&env, "USDC"),
            &7u32,
            &Some(usdc_issuer.clone()),
        );

        let xlm_meta = registry.get_asset_metadata(&xlm).unwrap();
        assert_eq!(xlm_meta.symbol, String::from_str(&env, "XLM"));
        assert_eq!(xlm_meta.decimals, 7);
        assert!(xlm_meta.issuer.is_none());

        let usdc_meta = registry.get_asset_metadata(&usdc).unwrap();
        assert_eq!(usdc_meta.symbol, String::from_str(&env, "USDC"));
        assert_eq!(usdc_meta.decimals, 7);
        assert_eq!(usdc_meta.issuer, Some(usdc_issuer));

        // Update USDC decimals (e.g. if USDC changes on Stellar)
        let new_issuer = Address::generate(&env);
        registry.update_asset(
            &admin,
            &usdc,
            &String::from_str(&env, "USDC"),
            &6u32,
            &Some(new_issuer.clone()),
        );

        let updated = registry.get_asset_metadata(&usdc).unwrap();
        assert_eq!(updated.decimals, 6);
        assert_eq!(updated.issuer, Some(new_issuer));
    }
}
