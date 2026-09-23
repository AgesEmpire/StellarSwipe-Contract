//! Reward vault: multi-asset accounting and batch claim flow.
//!
//! # Issue #1020 – Batch reward claim
//! Providers accumulate rewards across multiple epochs (reward buckets).
//! `batch_claim_rewards` aggregates all eligible entries in a single call,
//! marks each bucket as claimed (idempotence), and emits one aggregate event.
//!
//! # Issue #1022 – Multi-asset reward vault
//! Each reward bucket is denominated in a specific asset token.  The vault
//! tracks per-asset total deposits and per-provider per-asset balances so
//! balances are never mixed.  `deposit_reward` validates that the asset is
//! registered before accepting funds.

use soroban_sdk::{contracttype, token, Address, Env, Symbol, Vec};

use shared::event_topics as topics;

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum RewardKey {
    /// Registered reward asset addresses (Vec<Address>).
    SupportedAssets,
    /// Per-asset total deposited balance: (asset) → i128.
    AssetTotalDeposited(Address),
    /// Per-provider per-asset claimable balance: (provider, asset) → i128.
    ProviderAssetBalance(Address, Address),
    /// Monotonic reward-bucket counter.
    BucketCounter,
    /// Individual reward bucket: bucket_id → RewardBucket.
    Bucket(u64),
    /// Claimed flag per (provider, bucket_id) – present means already claimed.
    Claimed(Address, u64),
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single reward allocation for a provider in a specific asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardBucket {
    pub bucket_id: u64,
    pub provider: Address,
    pub asset: Address,
    pub amount: i128,
    pub epoch: u64,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum RewardVaultError {
    /// Asset is not in the supported-assets list.
    UnsupportedAsset,
    /// Reward amount must be > 0.
    InvalidAmount,
    /// Batch size is 0 or exceeds the maximum.
    BatchSizeInvalid,
}

const MAX_REWARD_BATCH: u32 = 100;

// ── Internal helpers ──────────────────────────────────────────────────────────

fn contract_topic(env: &Env) -> Symbol {
    Symbol::new(env, "stake_vault")
}

fn supported_assets(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&RewardKey::SupportedAssets)
        .unwrap_or_else(|| Vec::new(env))
}

fn is_supported(env: &Env, asset: &Address) -> bool {
    let assets = supported_assets(env);
    for i in 0..assets.len() {
        if assets.get(i).unwrap() == *asset {
            return true;
        }
    }
    false
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Admin: register a new reward asset.  Idempotent – re-adding an existing
/// asset is a no-op.
pub fn add_supported_asset(env: &Env, asset: Address) {
    if is_supported(env, &asset) {
        return;
    }
    let mut assets = supported_assets(env);
    assets.push_back(asset.clone());
    env.storage()
        .instance()
        .set(&RewardKey::SupportedAssets, &assets);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_REWARD_ASSET_ADDED()),
        asset,
    );
}

/// Returns all registered reward asset addresses.
pub fn get_supported_assets(env: &Env) -> Vec<Address> {
    supported_assets(env)
}

/// Deposit `amount` of `asset` as a reward bucket for `provider` in `epoch`.
///
/// The asset must be registered via `add_supported_asset` first.
/// Tokens are transferred from `depositor` into the contract.
pub fn deposit_reward(
    env: &Env,
    depositor: &Address,
    provider: Address,
    asset: Address,
    amount: i128,
    epoch: u64,
) -> Result<u64, RewardVaultError> {
    if !is_supported(env, &asset) {
        return Err(RewardVaultError::UnsupportedAsset);
    }
    if amount <= 0 {
        return Err(RewardVaultError::InvalidAmount);
    }

    // Assign bucket id.
    let bucket_id: u64 = env
        .storage()
        .instance()
        .get(&RewardKey::BucketCounter)
        .unwrap_or(0u64);
    let next_id = bucket_id.saturating_add(1);
    env.storage()
        .instance()
        .set(&RewardKey::BucketCounter, &next_id);

    let bucket = RewardBucket {
        bucket_id,
        provider: provider.clone(),
        asset: asset.clone(),
        amount,
        epoch,
    };
    env.storage()
        .persistent()
        .set(&RewardKey::Bucket(bucket_id), &bucket);

    // Update per-asset total.
    let prev_total: i128 = env
        .storage()
        .persistent()
        .get(&RewardKey::AssetTotalDeposited(asset.clone()))
        .unwrap_or(0);
    env.storage().persistent().set(
        &RewardKey::AssetTotalDeposited(asset.clone()),
        &prev_total.saturating_add(amount),
    );

    // Update per-provider per-asset balance.
    let prev_bal: i128 = env
        .storage()
        .persistent()
        .get(&RewardKey::ProviderAssetBalance(
            provider.clone(),
            asset.clone(),
        ))
        .unwrap_or(0);
    env.storage().persistent().set(
        &RewardKey::ProviderAssetBalance(provider.clone(), asset.clone()),
        &prev_bal.saturating_add(amount),
    );

    // Transfer tokens into vault.
    token::Client::new(env, &asset).transfer(depositor, env.current_contract_address(), &amount);

    env.events().publish(
        (contract_topic(env), topics::TOPIC_REWARD_DEPOSITED()),
        (provider, asset, amount, epoch, bucket_id),
    );

    Ok(bucket_id)
}

/// Provider: claim rewards from a batch of bucket IDs in a single call.
///
/// - Buckets already claimed are silently skipped (idempotence).
/// - Buckets belonging to a different provider are skipped.
/// - Returns the per-asset totals transferred.
///
/// Emits one `rwdbatch` event per asset that had a non-zero payout.
pub fn batch_claim_rewards(
    env: &Env,
    provider: &Address,
    bucket_ids: Vec<u64>,
) -> Result<Vec<(Address, i128)>, RewardVaultError> {
    let len = bucket_ids.len();
    if len == 0 || len > MAX_REWARD_BATCH {
        return Err(RewardVaultError::BatchSizeInvalid);
    }

    // Accumulate per-asset totals in a simple parallel-arrays structure
    // (no std HashMap in no_std).
    let mut asset_keys: Vec<Address> = Vec::new(env);
    let mut asset_totals: Vec<i128> = Vec::new(env);

    for i in 0..len {
        let bid = bucket_ids.get(i).unwrap();

        // Idempotence: skip already-claimed buckets.
        let claim_key = RewardKey::Claimed(provider.clone(), bid);
        if env.storage().persistent().has(&claim_key) {
            continue;
        }

        let bucket: RewardBucket = match env.storage().persistent().get(&RewardKey::Bucket(bid)) {
            Some(b) => b,
            None => continue,
        };

        // Only claim buckets belonging to this provider.
        if bucket.provider != *provider {
            continue;
        }

        // Validate asset is still supported.
        if !is_supported(env, &bucket.asset) {
            continue;
        }

        // Mark claimed before any transfer (CEI).
        env.storage().persistent().set(&claim_key, &true);

        // Reduce per-provider per-asset balance.
        let bal_key = RewardKey::ProviderAssetBalance(provider.clone(), bucket.asset.clone());
        let prev_bal: i128 = env.storage().persistent().get(&bal_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&bal_key, &prev_bal.saturating_sub(bucket.amount));

        // Accumulate into asset_totals.
        let mut found = false;
        for j in 0..asset_keys.len() {
            if asset_keys.get(j).unwrap() == bucket.asset {
                let cur = asset_totals.get(j).unwrap();
                asset_totals.set(j, cur.saturating_add(bucket.amount));
                found = true;
                break;
            }
        }
        if !found {
            asset_keys.push_back(bucket.asset.clone());
            asset_totals.push_back(bucket.amount);
        }
    }

    // Transfer and emit per asset.
    let mut results: Vec<(Address, i128)> = Vec::new(env);
    for i in 0..asset_keys.len() {
        let asset = asset_keys.get(i).unwrap();
        let total = asset_totals.get(i).unwrap();
        if total <= 0 {
            continue;
        }
        token::Client::new(env, &asset).transfer(&env.current_contract_address(), provider, &total);

        env.events().publish(
            (contract_topic(env), topics::TOPIC_REWARD_BATCH_CLAIMED()),
            (provider.clone(), asset.clone(), total),
        );

        results.push_back((asset, total));
    }

    Ok(results)
}

/// Returns the claimable balance for `provider` in `asset`.
pub fn get_provider_reward_balance(env: &Env, provider: &Address, asset: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&RewardKey::ProviderAssetBalance(
            provider.clone(),
            asset.clone(),
        ))
        .unwrap_or(0)
}

/// Returns the total deposited amount for `asset` across all providers.
pub fn get_asset_total_deposited(env: &Env, asset: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&RewardKey::AssetTotalDeposited(asset.clone()))
        .unwrap_or(0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as TestAddress;
    use soroban_sdk::{contract, contractimpl, Env};

    // Minimal token mock for testing.
    mod token_mock {
        use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, MuxedAddress};

        #[contracttype]
        pub enum DataKey {
            Balance(Address),
        }

        #[contract]
        pub struct MockToken;

        #[contractimpl]
        impl MockToken {
            pub fn mint(env: Env, to: Address, amount: i128) {
                let key = DataKey::Balance(to.clone());
                let bal: i128 = env.storage().instance().get(&key).unwrap_or(0);
                env.storage().instance().set(&key, &(bal + amount));
            }
        }

        impl token::Interface for MockToken {
            fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
                0
            }
            fn approve(_env: Env, _from: Address, _spender: Address, _amount: i128, _expiry: u32) {
            }
            fn balance(env: Env, id: Address) -> i128 {
                env.storage()
                    .instance()
                    .get(&DataKey::Balance(id))
                    .unwrap_or(0)
            }
            fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
                let fk = DataKey::Balance(from.clone());
                let to_addr = to.address();
                let tk = DataKey::Balance(to_addr.clone());
                let fb: i128 = env.storage().instance().get(&fk).unwrap_or(0);
                let tb: i128 = env.storage().instance().get(&tk).unwrap_or(0);
                env.storage().instance().set(&fk, &(fb - amount));
                env.storage().instance().set(&tk, &(tb + amount));
            }
            fn transfer_from(
                _env: Env,
                _spender: Address,
                _from: Address,
                _to: Address,
                _amount: i128,
            ) {
            }
            fn burn(_env: Env, _from: Address, _amount: i128) {}
            fn burn_from(_env: Env, _spender: Address, _from: Address, _amount: i128) {}
            fn decimals(_env: Env) -> u32 {
                7
            }
            fn name(env: Env) -> soroban_sdk::String {
                soroban_sdk::String::from_str(&env, "Mock")
            }
            fn symbol(env: Env) -> soroban_sdk::String {
                soroban_sdk::String::from_str(&env, "MCK")
            }
        }
    }

    #[contract]
    struct VaultContract;

    #[contractimpl]
    impl VaultContract {}

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let vault = env.register(VaultContract, ());
        (env, vault)
    }

    #[test]
    fn add_and_query_supported_asset() {
        let (env, vault) = setup();
        let asset = Address::generate(&env);
        env.as_contract(&vault, || {
            assert!(!is_supported(&env, &asset));
            add_supported_asset(&env, asset.clone());
            assert!(is_supported(&env, &asset));
            // Idempotent second add.
            add_supported_asset(&env, asset.clone());
            assert_eq!(get_supported_assets(&env).len(), 1);
        });
    }

    #[test]
    fn deposit_unsupported_asset_fails() {
        let (env, vault) = setup();
        let depositor = Address::generate(&env);
        let provider = Address::generate(&env);
        let asset = Address::generate(&env);
        env.as_contract(&vault, || {
            let err =
                deposit_reward(&env, &depositor, provider, asset, 1_000, 1).unwrap_err();
            assert_eq!(err, RewardVaultError::UnsupportedAsset);
        });
    }

    #[test]
    fn batch_claim_empty_bucket_list_fails() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        env.as_contract(&vault, || {
            let err =
                batch_claim_rewards(&env, &provider, Vec::new(&env)).unwrap_err();
            assert_eq!(err, RewardVaultError::BatchSizeInvalid);
        });
    }

    #[test]
    fn batch_claim_idempotent_double_claim() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let asset_addr = env.register(token_mock::MockToken, ());

        env.as_contract(&vault, || {
            add_supported_asset(&env, asset_addr.clone());
        });

        // Mint tokens to vault so transfer succeeds.
        env.as_contract(&asset_addr, || {
            token_mock::MockToken::mint(env.clone(), vault.clone(), 10_000);
        });

        // Deposit a bucket.
        let bid = env.as_contract(&vault, || {
            deposit_reward(
                &env,
                &vault, // depositor is vault itself (already has tokens)
                provider.clone(),
                asset_addr.clone(),
                5_000,
                1,
            )
            .unwrap()
        });

        // First claim.
        let mut ids = Vec::new(&env);
        ids.push_back(bid);
        let results = env
            .as_contract(&vault, || {
                batch_claim_rewards(&env, &provider, ids.clone())
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results.get(0).unwrap().1, 5_000);

        // Second claim of same bucket → no payout (idempotent).
        let results2 = env
            .as_contract(&vault, || batch_claim_rewards(&env, &provider, ids))
            .unwrap();
        assert_eq!(results2.len(), 0);
    }

    #[test]
    fn multi_asset_balances_tracked_independently() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let asset_a = env.register(token_mock::MockToken, ());
        let asset_b = env.register(token_mock::MockToken, ());

        env.as_contract(&vault, || {
            add_supported_asset(&env, asset_a.clone());
            add_supported_asset(&env, asset_b.clone());
        });

        env.as_contract(&asset_a, || {
            token_mock::MockToken::mint(env.clone(), vault.clone(), 1_000);
        });
        env.as_contract(&asset_b, || {
            token_mock::MockToken::mint(env.clone(), vault.clone(), 2_000);
        });

        env.as_contract(&vault, || {
            deposit_reward(&env, &vault, provider.clone(), asset_a.clone(), 1_000, 1).unwrap();
            deposit_reward(&env, &vault, provider.clone(), asset_b.clone(), 2_000, 1).unwrap();

            assert_eq!(
                get_provider_reward_balance(&env, &provider, &asset_a),
                1_000
            );
            assert_eq!(
                get_provider_reward_balance(&env, &provider, &asset_b),
                2_000
            );
            assert_eq!(get_asset_total_deposited(&env, &asset_a), 1_000);
            assert_eq!(get_asset_total_deposited(&env, &asset_b), 2_000);
        });
    }
}
