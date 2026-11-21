#!/usr/bin/env bash
# scaffold_contract.sh — Generate a new StellarSwipe contract crate pre-wired with
# shared Pausable, Initializable, and storage-trait boilerplate.
#
# Usage:
#   ./scripts/scaffold_contract.sh <contract_name>
#
# Example:
#   ./scripts/scaffold_contract.sh my_new_contract
#
# The script creates contracts/<contract_name>/{Cargo.toml,src/lib.rs,src/tests.rs}
# and adds the crate to the workspace Cargo.toml.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CONTRACTS_DIR="${WORKSPACE_DIR}/contracts"

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <contract_name>" >&2
  exit 1
fi

NAME="$1"
# Validate: lowercase letters, digits, underscores only
if ! [[ "$NAME" =~ ^[a-z][a-z0-9_]*$ ]]; then
  echo "Error: contract name must be lowercase snake_case (got: $NAME)" >&2
  exit 1
fi

CRATE_DIR="${CONTRACTS_DIR}/${NAME}"
if [[ -d "$CRATE_DIR" ]]; then
  echo "Error: directory already exists: $CRATE_DIR" >&2
  exit 1
fi

# Pascal-case for type names: my_contract → MyContract
PASCAL_NAME=$(echo "$NAME" | sed -E 's/(^|_)([a-z])/\u\2/g')

mkdir -p "${CRATE_DIR}/src"

# ── Cargo.toml ────────────────────────────────────────────────────────────────
cat > "${CRATE_DIR}/Cargo.toml" << TOML
[package]
name = "stellar-swipe-${NAME//_/-}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
soroban-sdk = { workspace = true }
stellar-swipe-common = { path = "../common" }

[dev-dependencies]
soroban-sdk = { workspace = true, features = ["testutils"] }
TOML

# ── src/lib.rs ────────────────────────────────────────────────────────────────
cat > "${CRATE_DIR}/src/lib.rs" << RUST
#![no_std]
//! ${NAME} contract — scaffolded by scaffold_contract.sh.
//! Pre-wired with Pausable, Initializable, and StorageTrait conventions.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

// ── Storage trait (CRUD boilerplate) ─────────────────────────────────────────

pub trait StorageTrait<K, V> {
    fn read(env: &Env, key: &K) -> Option<V>;
    fn write(env: &Env, key: &K, value: &V);
    fn remove(env: &Env, key: &K);
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Initialized,
    Paused,
    // Add contract-specific keys here.
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ${PASCAL_NAME}Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    ContractPaused = 4,
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct ${PASCAL_NAME}Contract;

#[contractimpl]
impl ${PASCAL_NAME}Contract {
    // ── Initializable ─────────────────────────────────────────────────────────

    /// One-time initialization guard.
    pub fn initialize(env: Env, admin: Address) -> Result<(), ${PASCAL_NAME}Error> {
        if env.storage().instance().has(&DataKey::Initialized) {
            return Err(${PASCAL_NAME}Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Paused, &false);
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "${NAME}"), Symbol::new(&env, "initialized")),
            admin,
        );
        Ok(())
    }

    // ── Pausable ──────────────────────────────────────────────────────────────

    pub fn pause(env: Env) -> Result<(), ${PASCAL_NAME}Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &true);
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "${NAME}"), Symbol::new(&env, "paused")),
            (),
        );
        Ok(())
    }

    pub fn unpause(env: Env) -> Result<(), ${PASCAL_NAME}Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "${NAME}"), Symbol::new(&env, "unpaused")),
            (),
        );
        Ok(())
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage().instance().get(&DataKey::Paused).unwrap_or(false)
    }

    // ── StorageTrait example: generic persistent read/write ──────────────────

    /// Write an arbitrary i128 value under a named key (demonstrates storage-trait pattern).
    pub fn storage_write(env: Env, key: Symbol, value: i128) -> Result<(), ${PASCAL_NAME}Error> {
        Self::require_not_paused(&env)?;
        env.storage().persistent().set(&key, &value);
        Ok(())
    }

    pub fn storage_read(env: Env, key: Symbol) -> Option<i128> {
        env.storage().persistent().get(&key)
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn require_admin(env: &Env) -> Result<Address, ${PASCAL_NAME}Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(${PASCAL_NAME}Error::NotInitialized)
    }

    fn require_not_paused(env: &Env) -> Result<(), ${PASCAL_NAME}Error> {
        if env.storage().instance().get::<_, bool>(&DataKey::Paused).unwrap_or(false) {
            Err(${PASCAL_NAME}Error::ContractPaused)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
RUST

# ── src/tests.rs ──────────────────────────────────────────────────────────────
cat > "${CRATE_DIR}/src/tests.rs" << RUST
#![cfg(test)]

use crate::{${PASCAL_NAME}Contract, ${PASCAL_NAME}ContractClient, ${PASCAL_NAME}Error};
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(${PASCAL_NAME}Contract, ());
    (env, contract_id, admin)
}

#[test]
fn initialize_sets_admin_and_emits_event() {
    let (env, id, admin) = setup();
    let client = ${PASCAL_NAME}ContractClient::new(&env, &id);
    client.initialize(&admin);
    assert!(!client.is_paused());
}

#[test]
fn double_initialize_fails() {
    let (env, id, admin) = setup();
    let client = ${PASCAL_NAME}ContractClient::new(&env, &id);
    client.initialize(&admin);
    assert_eq!(
        client.try_initialize(&admin),
        Err(Ok(${PASCAL_NAME}Error::AlreadyInitialized))
    );
}

#[test]
fn pause_and_unpause() {
    let (env, id, admin) = setup();
    let client = ${PASCAL_NAME}ContractClient::new(&env, &id);
    client.initialize(&admin);

    client.pause();
    assert!(client.is_paused());

    client.unpause();
    assert!(!client.is_paused());
}

#[test]
fn storage_write_read_roundtrip() {
    let (env, id, admin) = setup();
    let client = ${PASCAL_NAME}ContractClient::new(&env, &id);
    client.initialize(&admin);

    let key = Symbol::new(&env, "mykey");
    client.storage_write(&key, &42);
    assert_eq!(client.storage_read(&key), Some(42));
}

#[test]
fn write_blocked_when_paused() {
    let (env, id, admin) = setup();
    let client = ${PASCAL_NAME}ContractClient::new(&env, &id);
    client.initialize(&admin);
    client.pause();

    let key = Symbol::new(&env, "k");
    assert_eq!(
        client.try_storage_write(&key, &1),
        Err(Ok(${PASCAL_NAME}Error::ContractPaused))
    );
}
RUST

# ── Wire into workspace Cargo.toml ────────────────────────────────────────────
WORKSPACE_TOML="${WORKSPACE_DIR}/Cargo.toml"
# Add "contracts/<name>" to the members list if not already present
if grep -q "\"contracts/${NAME}\"" "$WORKSPACE_TOML"; then
  echo "Note: workspace already contains contracts/${NAME}"
else
  sed -i "s|members = \[|members = [\n  \"contracts/${NAME}\",|" "$WORKSPACE_TOML"
fi

echo ""
echo "✓ Scaffolded: ${CRATE_DIR}"
echo "  • Cargo.toml  — depends on soroban-sdk + stellar-swipe-common"
echo "  • src/lib.rs  — initialize/pause/unpause + storage_write/storage_read"
echo "  • src/tests.rs — starter tests"
echo ""
echo "Workspace Cargo.toml updated with: contracts/${NAME}"
echo ""
echo "Next steps:"
echo "  cd stellar-swipe && cargo test -p stellar-swipe-${NAME//_/-}"
