//! Reward vault: multi-asset accounting and batch claim flow.
//!
//! # Issue #1020 – Batch reward claim
//! Providers accumulate rewards across multiple epochs (reward buckets).
//! `batch_claim_rewards` aggregates all eligible entries in a single call,
//! marks each bucket as claimed, and emits one aggregate event per asset.
//!
//! # Issue #1326 – Bounded, atomic batch claims
//! * A batch holds at most [`MAX_REWARD_BATCH`] bucket ids.  The length is
//!   checked before any bucket is read, so an oversized batch costs nothing.
//! * Claims are all-or-nothing.  Every id is validated first (it must exist,
//!   belong to the caller, be unclaimed, appear once in the batch and be in a
//!   supported asset) and the per-asset totals are summed with checked
//!   arithmetic, all before any write.  One bad id fails the whole batch and
//!   consumes nothing.
//! * Only then are entitlements committed (claimed flags and balances, before
//!   any transfer) and the per-asset transfers made.  A failed transfer
//!   returns an error, and the host then reverts every write and transfer made
//!   earlier in the same call.
//! * A bucket can only be claimed once: re-claiming it is rejected with
//!   [`RewardVaultError::AlreadyClaimed`] instead of being skipped.
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
    /// Batch size is 0 or exceeds [`MAX_REWARD_BATCH`].
    BatchSizeInvalid,
    /// A bucket id in the batch does not exist.
    BucketNotFound,
    /// A bucket in the batch belongs to a different provider.
    NotBucketOwner,
    /// A bucket in the batch has already been claimed.
    AlreadyClaimed,
    /// The same bucket id appears more than once in the batch.
    DuplicateBucket,
    /// A batch total or the remaining balance falls outside `i128`.
    Overflow,
    /// A reward token transfer failed; the whole call is reverted.
    Token(shared::TokenFailure),
}

/// Maximum number of bucket ids accepted by one [`batch_claim_rewards`] call
/// (Issue #1326), checked before any bucket is read.
///
/// Each id costs two persistent reads (the bucket and its claimed flag) and
/// one write (the claimed flag); each distinct asset adds a balance write and
/// a token transfer.  The cap matches the reward-campaign batch limit
/// (`reward_campaign::MAX_CAMPAIGN_BATCH`), so a full batch stays within the
/// same per-call resource bound.
pub const MAX_REWARD_BATCH: u32 = 50;

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

pub(crate) fn is_supported(env: &Env, asset: &Address) -> bool {
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
/// All-or-nothing (Issue #1326): the batch must hold 1..=[`MAX_REWARD_BATCH`]
/// ids, and every id must name an existing, unclaimed bucket owned by
/// `provider` in a supported asset, with no id repeated.  Otherwise the call
/// fails and no entitlement is consumed.
///
/// Returns the per-asset totals transferred, in first-seen asset order.
/// Emits one `rwdbatch` event per asset.
pub fn batch_claim_rewards(
    env: &Env,
    provider: &Address,
    bucket_ids: Vec<u64>,
) -> Result<Vec<(Address, i128)>, RewardVaultError> {
    let len = bucket_ids.len();
    if len == 0 || len > MAX_REWARD_BATCH {
        return Err(RewardVaultError::BatchSizeInvalid);
    }

    // ── Phase 1: validate every id and total per asset (reads only) ─────────
    // Parallel arrays stand in for a map (no std HashMap in no_std).
    let supported = supported_assets(env);
    let mut asset_keys: Vec<Address> = Vec::new(env);
    let mut asset_totals: Vec<i128> = Vec::new(env);

    for i in 0..len {
        let bid = bucket_ids.get_unchecked(i);
        for j in 0..i {
            if bucket_ids.get_unchecked(j) == bid {
                return Err(RewardVaultError::DuplicateBucket);
            }
        }

        let bucket: RewardBucket = env
            .storage()
            .persistent()
            .get(&RewardKey::Bucket(bid))
            .ok_or(RewardVaultError::BucketNotFound)?;
        if bucket.provider != *provider {
            return Err(RewardVaultError::NotBucketOwner);
        }
        if env
            .storage()
            .persistent()
            .has(&RewardKey::Claimed(provider.clone(), bid))
        {
            return Err(RewardVaultError::AlreadyClaimed);
        }
        if !supported.contains(&bucket.asset) {
            return Err(RewardVaultError::UnsupportedAsset);
        }

        match asset_keys.first_index_of(&bucket.asset) {
            Some(j) => {
                let total = asset_totals
                    .get_unchecked(j)
                    .checked_add(bucket.amount)
                    .ok_or(RewardVaultError::Overflow)?;
                asset_totals.set(j, total);
            }
            None => {
                asset_keys.push_back(bucket.asset);
                asset_totals.push_back(bucket.amount);
            }
        }
    }

    let mut new_balances: Vec<i128> = Vec::new(env);
    for i in 0..asset_keys.len() {
        let bal_key =
            RewardKey::ProviderAssetBalance(provider.clone(), asset_keys.get_unchecked(i));
        let prev_bal: i128 = env.storage().persistent().get(&bal_key).unwrap_or(0);
        let new_bal = prev_bal
            .checked_sub(asset_totals.get_unchecked(i))
            .filter(|b| *b >= 0)
            .ok_or(RewardVaultError::Overflow)?;
        new_balances.push_back(new_bal);
    }

    // ── Phase 2: commit entitlements before any transfer (CEI) ──────────────
    for i in 0..len {
        env.storage().persistent().set(
            &RewardKey::Claimed(provider.clone(), bucket_ids.get_unchecked(i)),
            &true,
        );
    }
    for i in 0..asset_keys.len() {
        env.storage().persistent().set(
            &RewardKey::ProviderAssetBalance(provider.clone(), asset_keys.get_unchecked(i)),
            &new_balances.get_unchecked(i),
        );
    }

    // ── Phase 3: transfer and emit per asset ────────────────────────────────
    // A failed transfer returns `Err`, and the host reverts the writes above
    // together with any transfer already made in this call.
    let mut results: Vec<(Address, i128)> = Vec::new(env);
    for i in 0..asset_keys.len() {
        let asset = asset_keys.get_unchecked(i);
        let total = asset_totals.get_unchecked(i);
        shared::token_error::map_result(token::Client::new(env, &asset).try_transfer(
            &env.current_contract_address(),
            provider,
            &total,
        ))
        .map_err(RewardVaultError::Token)?;

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
        use soroban_sdk::token::Interface as _;

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

        #[contractimpl]
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
    fn batch_claim_rejects_already_claimed_bucket() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let asset = funded_asset(&env, &vault, 10_000);
        let bid = deposit(&env, &vault, &provider, &asset, 5_000);
        let ids = Vec::from_array(&env, [bid]);

        let results = env
            .as_contract(&vault, || batch_claim_rewards(&env, &provider, ids.clone()))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results.get(0).unwrap().1, 5_000);
        assert_eq!(token_balance(&env, &asset, &provider), 5_000);

        // Re-claiming pays nothing and is reported, not silently skipped.
        let err = env
            .as_contract(&vault, || batch_claim_rewards(&env, &provider, ids))
            .unwrap_err();
        assert_eq!(err, RewardVaultError::AlreadyClaimed);
        assert_eq!(token_balance(&env, &asset, &provider), 5_000);
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

    // ── Issue #1326: bounded, atomic batch claims ────────────────────────────

    fn funded_asset(env: &Env, vault: &Address, amount: i128) -> Address {
        let asset = env.register(token_mock::MockToken, ());
        env.as_contract(vault, || add_supported_asset(env, asset.clone()));
        env.as_contract(&asset, || {
            token_mock::MockToken::mint(env.clone(), vault.clone(), amount);
        });
        asset
    }

    fn deposit(
        env: &Env,
        vault: &Address,
        provider: &Address,
        asset: &Address,
        amount: i128,
    ) -> u64 {
        env.as_contract(vault, || {
            deposit_reward(env, vault, provider.clone(), asset.clone(), amount, 1).unwrap()
        })
    }

    fn token_balance(env: &Env, asset: &Address, who: &Address) -> i128 {
        token::Client::new(env, asset).balance(who)
    }

    fn is_claimed(env: &Env, vault: &Address, provider: &Address, bid: u64) -> bool {
        env.as_contract(vault, || {
            env.storage()
                .persistent()
                .has(&RewardKey::Claimed(provider.clone(), bid))
        })
    }

    /// Fails `batch` and asserts that nothing at all was consumed.
    fn assert_rejected_without_effect(
        env: &Env,
        vault: &Address,
        provider: &Address,
        asset: &Address,
        batch: Vec<u64>,
        expected: RewardVaultError,
    ) {
        let entitled = env.as_contract(vault, || get_provider_reward_balance(env, provider, asset));
        let paid = token_balance(env, asset, provider);
        let err = env
            .as_contract(vault, || batch_claim_rewards(env, provider, batch.clone()))
            .unwrap_err();
        assert_eq!(err, expected);
        assert_eq!(
            env.as_contract(vault, || get_provider_reward_balance(env, provider, asset)),
            entitled
        );
        assert_eq!(token_balance(env, asset, provider), paid);
        for bid in batch.iter() {
            assert!(!is_claimed(env, vault, provider, bid));
        }
    }

    #[test]
    fn max_batch_is_documented_bound() {
        assert_eq!(MAX_REWARD_BATCH, crate::reward_campaign::MAX_CAMPAIGN_BATCH);
    }

    #[test]
    fn batch_claim_at_maximum_size_claims_everything() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let asset = funded_asset(&env, &vault, 1_000_000);
        let mut ids = Vec::new(&env);
        for i in 0..MAX_REWARD_BATCH {
            ids.push_back(deposit(&env, &vault, &provider, &asset, 100 + i as i128));
        }
        let expected: i128 = (0..MAX_REWARD_BATCH as i128).map(|i| 100 + i).sum();

        let results = env
            .as_contract(&vault, || batch_claim_rewards(&env, &provider, ids.clone()))
            .unwrap();
        assert_eq!(results, Vec::from_array(&env, [(asset.clone(), expected)]));
        assert_eq!(token_balance(&env, &asset, &provider), expected);
        assert_eq!(
            env.as_contract(&vault, || get_provider_reward_balance(
                &env, &provider, &asset
            )),
            0
        );
        for bid in ids.iter() {
            assert!(is_claimed(&env, &vault, &provider, bid));
        }
    }

    #[test]
    fn batch_claim_over_limit_fails_before_processing() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let asset = funded_asset(&env, &vault, 1_000_000);
        let mut ids = Vec::new(&env);
        for _ in 0..MAX_REWARD_BATCH {
            ids.push_back(deposit(&env, &vault, &provider, &asset, 10));
        }
        // The extra id does not exist: had any bucket been processed, the
        // error would be `BucketNotFound` rather than `BatchSizeInvalid`.
        ids.push_back(u64::MAX);
        assert_rejected_without_effect(
            &env,
            &vault,
            &provider,
            &asset,
            ids,
            RewardVaultError::BatchSizeInvalid,
        );
    }

    #[test]
    fn batch_claim_duplicate_id_fails_whole_batch() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let asset = funded_asset(&env, &vault, 10_000);
        let a = deposit(&env, &vault, &provider, &asset, 1_000);
        let b = deposit(&env, &vault, &provider, &asset, 2_000);

        assert_rejected_without_effect(
            &env,
            &vault,
            &provider,
            &asset,
            Vec::from_array(&env, [a, b, a]),
            RewardVaultError::DuplicateBucket,
        );

        // The same buckets remain claimable exactly once.
        let results = env
            .as_contract(&vault, || {
                batch_claim_rewards(&env, &provider, Vec::from_array(&env, [a, b]))
            })
            .unwrap();
        assert_eq!(results.get(0).unwrap().1, 3_000);
    }

    #[test]
    fn batch_claim_one_invalid_item_consumes_nothing() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let other = Address::generate(&env);
        let asset = funded_asset(&env, &vault, 100_000);
        let valid_a = deposit(&env, &vault, &provider, &asset, 1_000);
        let valid_b = deposit(&env, &vault, &provider, &asset, 2_000);
        let foreign = deposit(&env, &vault, &other, &asset, 4_000);
        let spent = deposit(&env, &vault, &provider, &asset, 500);
        env.as_contract(&vault, || {
            batch_claim_rewards(&env, &provider, Vec::from_array(&env, [spent])).unwrap()
        });

        // The invalid item sits last, after valid ones were already seen.
        let cases = [
            (u64::MAX, RewardVaultError::BucketNotFound),
            (foreign, RewardVaultError::NotBucketOwner),
            (spent, RewardVaultError::AlreadyClaimed),
        ];
        for (bad, expected) in cases {
            let entitled = env.as_contract(&vault, || {
                get_provider_reward_balance(&env, &provider, &asset)
            });
            let paid = token_balance(&env, &asset, &provider);
            let err = env
                .as_contract(&vault, || {
                    batch_claim_rewards(
                        &env,
                        &provider,
                        Vec::from_array(&env, [valid_a, valid_b, bad]),
                    )
                })
                .unwrap_err();
            assert_eq!(err, expected);
            assert!(!is_claimed(&env, &vault, &provider, valid_a));
            assert!(!is_claimed(&env, &vault, &provider, valid_b));
            assert_eq!(
                env.as_contract(&vault, || get_provider_reward_balance(
                    &env, &provider, &asset
                )),
                entitled
            );
            assert_eq!(token_balance(&env, &asset, &provider), paid);
        }
        // The other provider's entitlement is untouched too.
        assert!(!is_claimed(&env, &vault, &other, foreign));
        assert_eq!(
            env.as_contract(&vault, || get_provider_reward_balance(&env, &other, &asset)),
            4_000
        );
    }

    #[test]
    fn batch_claim_total_overflow_fails_before_any_write() {
        let (env, vault) = setup();
        let provider = Address::generate(&env);
        let asset = funded_asset(&env, &vault, 0);
        // Seeded directly: no token can hold i128::MAX + 1 to deposit them.
        let (a, b) = (1_000u64, 1_001u64);
        env.as_contract(&vault, || {
            for (bucket_id, amount) in [(a, i128::MAX), (b, 1)] {
                let bucket = RewardBucket {
                    bucket_id,
                    provider: provider.clone(),
                    asset: asset.clone(),
                    amount,
                    epoch: 1,
                };
                env.storage()
                    .persistent()
                    .set(&RewardKey::Bucket(bucket_id), &bucket);
            }
        });
        let err = env
            .as_contract(&vault, || {
                batch_claim_rewards(&env, &provider, Vec::from_array(&env, [a, b]))
            })
            .unwrap_err();
        assert_eq!(err, RewardVaultError::Overflow);
        assert!(!is_claimed(&env, &vault, &provider, a));
        assert!(!is_claimed(&env, &vault, &provider, b));
    }
}
