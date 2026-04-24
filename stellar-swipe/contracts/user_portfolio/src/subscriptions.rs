//! On-chain premium feed subscriptions: provider-set pricing, user-paid renewal, verifiable access.

use soroban_sdk::{contracterror, contracttype, symbol_short, token, Address, Env};

use crate::storage::DataKey;

/// Wall-clock seconds for one calendar day (subscription length uses days → timestamp).
pub const SECONDS_PER_DAY: u64 = 86_400;

/// Upper bound on `duration_days` for one `subscribe_to_provider` call.
pub const MAX_SUBSCRIPTION_DAYS: u32 = 366 * 5;

/// ~1 day in ledgers (5s slot) — used only for persistent storage TTL bumps.
const LEDGERS_PER_DAY: u32 = 17_280;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionRecord {
    pub expires_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderSubscriptionTerms {
    /// SEP-41 token contract (native XLM SAC or USDC).
    pub fee_token: Address,
    /// Fee in token smallest units charged per calendar day subscribed.
    pub fee_per_day: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    /// Active subscription for (`user`, `provider`).
    Subscription(Address, Address),
    /// Fee schedule published by `provider`.
    ProviderTerms(Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SubscriptionError {
    NotInitialized = 1,
    NoTermsFromProvider = 2,
    InvalidDuration = 3,
    Overflow = 4,
    InvalidFee = 5,
    SelfSubscribe = 6,
}

fn require_portfolio_initialized(env: &Env) -> Result<(), SubscriptionError> {
    if !env.storage().instance().has(&DataKey::Initialized) {
        return Err(SubscriptionError::NotInitialized);
    }
    Ok(())
}

fn extend_persistent_subscription_key(env: &Env, key: &StorageKey, duration_days: u32) {
    let extend_to = LEDGERS_PER_DAY.saturating_mul(duration_days.saturating_add(30));
    let threshold = extend_to / 2;
    env.storage()
        .persistent()
        .extend_ttl(key, threshold, extend_to);
}

/// Provider publishes [`ProviderSubscriptionTerms`] (fee token + stroops per day). Callable only by `provider`.
pub fn set_provider_subscription_terms(
    env: &Env,
    provider: &Address,
    fee_token: Address,
    fee_per_day: i128,
) -> Result<(), SubscriptionError> {
    provider.require_auth();
    require_portfolio_initialized(env)?;
    if fee_per_day <= 0 {
        return Err(SubscriptionError::InvalidFee);
    }
    let terms = ProviderSubscriptionTerms {
        fee_token,
        fee_per_day,
    };
    let key = StorageKey::ProviderTerms(provider.clone());
    env.storage().persistent().set(&key, &terms);
    extend_persistent_subscription_key(env, &key, MAX_SUBSCRIPTION_DAYS);
    Ok(())
}

/// User pays `fee_per_day * duration_days` to `provider` and extends subscription expiry.
pub fn subscribe_to_provider(
    env: &Env,
    user: &Address,
    provider: &Address,
    duration_days: u32,
) -> Result<(), SubscriptionError> {
    user.require_auth();
    require_portfolio_initialized(env)?;
    if user == provider {
        return Err(SubscriptionError::SelfSubscribe);
    }
    if duration_days == 0 || duration_days > MAX_SUBSCRIPTION_DAYS {
        return Err(SubscriptionError::InvalidDuration);
    }
    let terms: ProviderSubscriptionTerms = env
        .storage()
        .persistent()
        .get(&StorageKey::ProviderTerms(provider.clone()))
        .ok_or(SubscriptionError::NoTermsFromProvider)?;
    let total = terms
        .fee_per_day
        .checked_mul(duration_days as i128)
        .ok_or(SubscriptionError::Overflow)?;
    if total <= 0 {
        return Err(SubscriptionError::Overflow);
    }

    token::Client::new(env, &terms.fee_token).transfer(user, provider, &total);

    let now = env.ledger().timestamp();
    let sub_key = StorageKey::Subscription(user.clone(), provider.clone());
    let base = match env.storage().persistent().get::<_, SubscriptionRecord>(&sub_key) {
        Some(rec) if rec.expires_at > now => rec.expires_at,
        _ => now,
    };
    let add_secs = (duration_days as u64)
        .checked_mul(SECONDS_PER_DAY)
        .ok_or(SubscriptionError::Overflow)?;
    let expires_at = base
        .checked_add(add_secs)
        .ok_or(SubscriptionError::Overflow)?;

    let record = SubscriptionRecord { expires_at };
    env.storage().persistent().set(&sub_key, &record);
    extend_persistent_subscription_key(env, &sub_key, duration_days);

    shared::events::emit_subscription_created(
        env,
        shared::events::EvtSubscriptionCreated {
            user: user.clone(),
            provider: provider.clone(),
            expires_at,
        },
    );

    Ok(())
}

/// Returns true when `user` has a non-expired subscription to `provider`.
pub fn check_subscription(env: &Env, user: &Address, provider: &Address) -> bool {
    if !env.storage().instance().has(&DataKey::Initialized) {
        return false;
    }
    let sub_key = StorageKey::Subscription(user.clone(), provider.clone());
    let Some(rec) = env
        .storage()
        .persistent()
        .get::<_, SubscriptionRecord>(&sub_key)
    else {
        return false;
    };
    env.ledger().timestamp() < rec.expires_at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserPortfolio, UserPortfolioClient};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::{Address, Env};

    fn sac_token(env: &Env) -> Address {
        let issuer = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(issuer);
        sac.address()
    }

    fn setup() -> (Env, Address, Address, Address, Address, Address, UserPortfolioClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let provider = Address::generate(&env);
        let subscriber = Address::generate(&env);
        let other = Address::generate(&env);

        let oracle = Address::generate(&env);
        #[allow(deprecated)]
        let portfolio_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        client.initialize(&admin, &oracle);

        let token = sac_token(&env);
        StellarAssetClient::new(&env, &token).mint(&subscriber, &1_000_000_000i128);
        StellarAssetClient::new(&env, &token).mint(&other, &1_000_000_000i128);

        (
            env,
            admin,
            provider,
            subscriber,
            other,
            token,
            client,
        )
    }

    #[test]
    fn active_subscription_allows_check() {
        let (_env, _admin, provider, subscriber, _other, token, client) = setup();
        assert!(client
            .try_set_provider_subscription_terms(&provider, &token, &100_000i128)
            .is_ok());
        assert!(client
            .try_subscribe_to_provider(&subscriber, &provider, &30u32)
            .is_ok());

        assert!(client.check_subscription(&subscriber, &provider));
    }

    #[test]
    fn expired_subscription_denies_check() {
        let (env, _admin, provider, subscriber, _other, token, client) = setup();
        assert!(client
            .try_set_provider_subscription_terms(&provider, &token, &50_000i128)
            .is_ok());
        assert!(client
            .try_subscribe_to_provider(&subscriber, &provider, &1u32)
            .is_ok());
        assert!(client.check_subscription(&subscriber, &provider));

        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_DAY + 1;
        });

        assert!(!client.check_subscription(&subscriber, &provider));
    }

    #[test]
    fn non_subscriber_denied() {
        let (_env, _admin, provider, subscriber, other, token, client) = setup();
        assert!(client
            .try_set_provider_subscription_terms(&provider, &token, &10_000i128)
            .is_ok());
        assert!(client
            .try_subscribe_to_provider(&subscriber, &provider, &7u32)
            .is_ok());

        assert!(!client.check_subscription(&other, &provider));
    }

    #[test]
    fn provider_receives_fee() {
        let (env, _admin, provider, subscriber, _other, token, client) = setup();
        assert!(client
            .try_set_provider_subscription_terms(&provider, &token, &25_000i128)
            .is_ok());
        let before = StellarAssetClient::new(&env, &token).balance(&provider);
        assert!(client
            .try_subscribe_to_provider(&subscriber, &provider, &4u32)
            .is_ok());
        let after = StellarAssetClient::new(&env, &token).balance(&provider);
        assert_eq!(after - before, 100_000i128);
    }
}
