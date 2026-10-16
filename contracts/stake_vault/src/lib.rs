//! Stake vault contract.
//!
//! Holds delegated stake per provider and enforces a per-provider
//! concentration limit so that no single provider can accumulate an
//! unbounded share of the total delegated stake.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

/// Storage keys used by the stake vault.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Address of the admin allowed to update configuration.
    Admin,
    /// Maximum share of total stake a single provider may hold, in basis
    /// points (1 bps = 0.01%). `10_000` means 100% (no limit).
    MaxProviderShareBps,
    /// Total amount of stake currently delegated across all providers.
    TotalStake,
    /// Amount of stake currently delegated to a given provider.
    ProviderStake(Address),
    /// Emergency pause flag (issue #1266). When `true`, new delegations are
    /// rejected while withdrawals remain available.
    Paused,
}

/// Errors returned by the stake vault.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VaultError {
    /// Caller is not the configured admin.
    NotAdmin = 1,
    /// Delegation would push a provider above the concentration limit.
    ConcentrationLimitExceeded = 2,
    /// Amount must be strictly positive.
    InvalidAmount = 3,
    /// Withdrawal exceeds the provider's delegated stake.
    InsufficientStake = 4,
    /// Configured share is outside the valid basis-point range.
    InvalidShare = 5,
    /// New delegations are rejected while the vault is paused (issue #1266).
    VaultPaused = 6,
    /// An arithmetic operation on stake totals or the concentration comparison
    /// overflowed its representable range. No state was mutated.
    ArithmeticOverflow = 7,
}

/// Auditable event emitted when the emergency pause state changes (issue #1266).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseChanged {
    pub paused: bool,
    pub actor: Address,
}

/// Structured payload for stake lifecycle events (issue #1262).
///
/// Carries the operation, the provider whose position changed, the amount
/// applied, and the resulting provider and total balances so indexers can
/// rebuild accounting from the event stream alone.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeChangedEvent {
    pub operation: Symbol,
    pub provider: Address,
    pub amount: i128,
    pub provider_balance: i128,
    pub total_balance: i128,
}

#[contract]
pub struct StakeVault;

#[contractimpl]
impl StakeVault {
    /// Initialise the vault with an admin and a per-provider concentration
    /// limit expressed in basis points of total delegated stake.
    ///
    /// `max_provider_share_bps` must be in `1..=10_000`.
    pub fn initialize(env: Env, admin: Address, max_provider_share_bps: u32) -> Result<(), VaultError> {
        if max_provider_share_bps == 0 || max_provider_share_bps > 10_000 {
            return Err(VaultError::InvalidShare);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::MaxProviderShareBps, &max_provider_share_bps);
        env.storage().instance().set(&DataKey::TotalStake, &0i128);
        Ok(())
    }

    /// Update the per-provider concentration limit.
    ///
    /// Policy: configuration updates are forward-looking only. Lowering the
    /// limit never forces existing positions to unwind; it only prevents new
    /// delegations that would breach the new limit. Existing providers that
    /// are already above the limit may still withdraw, and may not receive
    /// further delegations until their share falls back under the limit.
    pub fn set_max_provider_share_bps(env: Env, max_provider_share_bps: u32) -> Result<(), VaultError> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if max_provider_share_bps == 0 || max_provider_share_bps > 10_000 {
            return Err(VaultError::InvalidShare);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxProviderShareBps, &max_provider_share_bps);
        Ok(())
    }

    /// Issue #1266: change the emergency pause state.
    ///
    /// Only the authorized administrator can toggle the pause. While paused new
    /// delegations are rejected, but withdrawals stay available so providers
    /// always retain a path to exit. The change emits an auditable event.
    pub fn set_paused(env: Env, paused: bool) -> Result<(), VaultError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(VaultError::NotAdmin)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &paused);
        env.events().publish(
            (Symbol::new(&env, "pause_changed"),),
            PauseChanged {
                paused,
                actor: admin,
            },
        );
        Ok(())
    }

    /// Issue #1266: report whether new delegations are currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Delegate `amount` of stake to `provider`.
    ///
    /// The concentration limit is enforced *before* any state is mutated:
    /// if the resulting provider share would exceed the configured limit the
    /// call fails and no balances change.
    pub fn delegate(env: Env, provider: Address, amount: i128) -> Result<(), VaultError> {
        if amount <= 0 {
            return Err(VaultError::InvalidAmount);
        }

        // Issue #1266: reject new delegations while the emergency pause is
        // active. Withdrawals are intentionally unaffected.
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            return Err(VaultError::VaultPaused);
        }

        let total: i128 = env.storage().instance().get(&DataKey::TotalStake).unwrap_or(0);
        let current: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ProviderStake(provider.clone()))
            .unwrap_or(0);

        // Issue #1261: use checked arithmetic and fail before any persistent
        // write when a value cannot be represented.
        let new_total = total
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        let new_provider = current
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;

        // Enforce the concentration limit before mutating any state.
        let max_bps: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxProviderShareBps)
            .unwrap_or(10_000);
        if !Self::within_limit(new_provider, new_total, max_bps)? {
            return Err(VaultError::ConcentrationLimitExceeded);
        }

        env.storage().instance().set(&DataKey::TotalStake, &new_total);
        env.storage()
            .persistent()
            .set(&DataKey::ProviderStake(provider.clone()), &new_provider);

        // Issue #1262: emit an auditable delegation event so indexers can track
        // stake changes without reconstructing every provider balance.
        Self::emit_stake_changed(
            &env,
            Symbol::new(&env, "stake_delegated"),
            provider,
            amount,
            new_provider,
            new_total,
        );
        Ok(())
    }

    /// Withdraw `amount` of stake from `provider`.
    ///
    /// Withdrawals always reduce concentration and are therefore never
    /// blocked by the limit; they may free capacity for further delegations.
    pub fn withdraw(env: Env, provider: Address, amount: i128) -> Result<(), VaultError> {
        if amount <= 0 {
            return Err(VaultError::InvalidAmount);
        }

        // Issue #1258: the owner of the position must authorize before any
        // stake is reduced; unauthorized callers leave state untouched.
        provider.require_auth();

        let current: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ProviderStake(provider.clone()))
            .unwrap_or(0);
        if amount > current {
            return Err(VaultError::InsufficientStake);
        }

        let total: i128 = env.storage().instance().get(&DataKey::TotalStake).unwrap_or(0);
        let new_provider = current - amount;
        let new_total = total - amount;
        env.storage()
            .instance()
            .set(&DataKey::TotalStake, &new_total);

        // Issue #1263: drop the persistent entry entirely once the position is
        // empty so inactive providers are not stored as zero-balance records.
        // Partial withdrawals retain the entry.
        if new_provider == 0 {
            env.storage()
                .persistent()
                .remove(&DataKey::ProviderStake(provider.clone()));
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::ProviderStake(provider.clone()), &new_provider);
        }

        // Issue #1262: emit an auditable withdrawal event naming the authorized
        // provider and the resulting balances.
        Self::emit_stake_changed(
            &env,
            Symbol::new(&env, "stake_withdrawn"),
            provider,
            amount,
            new_provider,
            new_total,
        );
        Ok(())
    }

    /// Emit a structured stake lifecycle event (issue #1262).
    ///
    /// The payload always carries the provider, the amount applied, the
    /// operation topic, and the resulting provider and total balances so
    /// downstream consumers can rebuild accounting from the event stream.
    fn emit_stake_changed(
        env: &Env,
        operation: Symbol,
        provider: Address,
        amount: i128,
        provider_balance: i128,
        total_balance: i128,
    ) {
        env.events().publish(
            (operation.clone(),),
            StakeChangedEvent {
                operation,
                provider,
                amount,
                provider_balance,
                total_balance,
            },
        );
    }

    /// Returns the stake currently delegated to `provider`.
    pub fn provider_stake(env: Env, provider: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::ProviderStake(provider))
            .unwrap_or(0)
    }

    /// Returns the total stake delegated across all providers.
    pub fn total_stake(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalStake).unwrap_or(0)
    }

    /// Returns `true` when `provider_stake / total_stake` is within `max_bps`.
    ///
    /// Issue #1261: both sides of the comparison are computed with checked
    /// multiplication so an unrepresentable intermediate cannot silently wrap
    /// into an unsafe result.
    fn within_limit(
        provider_stake: i128,
        total_stake: i128,
        max_bps: u32,
    ) -> Result<bool, VaultError> {
        if total_stake <= 0 {
            return Ok(true);
        }
        // provider_stake / total_stake <= max_bps / 10_000
        let left = provider_stake
            .checked_mul(10_000)
            .ok_or(VaultError::ArithmeticOverflow)?;
        let right = total_stake
            .checked_mul(max_bps as i128)
            .ok_or(VaultError::ArithmeticOverflow)?;
        Ok(left <= right)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::{Env, TryFromVal};

    fn setup(max_bps: u32) -> (Env, StakeVaultClient<'static>, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, StakeVault);
        let client = StakeVaultClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);
        client.initialize(&admin, &max_bps);
        (env, client, admin, provider)
    }

    #[test]
    fn delegate_at_exact_limit_is_allowed() {
        // 50% limit: delegating 500 out of a resulting total of 1000 is exact.
        let (_env, client, _admin, provider) = setup(5_000);
        client.delegate(&provider, &500);
        assert_eq!(client.provider_stake(&provider), 500);
        assert_eq!(client.total_stake(), 500);
    }

    #[test]
    fn delegate_over_limit_is_rejected_without_state_change() {
        let (_env, client, _admin, provider) = setup(5_000);
        // First delegation is fine (100% of total).
        client.delegate(&provider, &500);
        // Second delegation would make the provider 1000/1000 = 100% > 50%.
        let result = client.try_delegate(&provider, &500);
        assert_eq!(result, Err(Ok(VaultError::ConcentrationLimitExceeded)));
        // State is unchanged because the check runs before mutation.
        assert_eq!(client.provider_stake(&provider), 500);
        assert_eq!(client.total_stake(), 500);
    }

    #[test]
    fn withdrawal_reduces_concentration_and_frees_capacity() {
        let (_env, client, _admin, provider) = setup(5_000);
        client.delegate(&provider, &500);
        // Over-limit delegation is rejected.
        assert!(client.try_delegate(&provider, &500).is_err());
        // Withdrawing reduces concentration.
        client.withdraw(&provider, &250);
        assert_eq!(client.provider_stake(&provider), 250);
        assert_eq!(client.total_stake(), 250);
        // Capacity is freed: the provider can delegate again up to the limit.
        client.delegate(&provider, &250);
        assert_eq!(client.provider_stake(&provider), 500);
        assert_eq!(client.total_stake(), 500);
    }

    #[test]
    fn lowering_limit_does_not_force_unwind_but_blocks_new_delegation() {
        let (_env, client, _admin, provider) = setup(10_000);
        client.delegate(&provider, &1_000);
        // Tighten the limit; existing position is untouched.
        client.set_max_provider_share_bps(&5_000);
        assert_eq!(client.provider_stake(&provider), 1_000);
        // New delegation that would breach the new limit is rejected.
        assert_eq!(
            client.try_delegate(&provider, &1),
            Err(Ok(VaultError::ConcentrationLimitExceeded))
        );
        // Withdrawal still works.
        client.withdraw(&provider, &500);
        assert_eq!(client.provider_stake(&provider), 500);
    }

    #[test]
    fn pause_blocks_delegations_but_allows_withdrawals() {
        // Issue #1266: an authorized operator can pause new delegations while
        // providers retain a path to exit. `env.events().all()` reflects the
        // most recent invocation, so each toggle's event is captured right
        // after that call.
        let (env, client, admin, provider) = setup(10_000);
        client.delegate(&provider, &1_000);
        assert!(!client.is_paused());

        client.set_paused(&true);
        let pause_on_events = env.events().all();
        assert!(client.is_paused());

        // New delegation is rejected while paused and state is unchanged.
        assert_eq!(
            client.try_delegate(&provider, &1),
            Err(Ok(VaultError::VaultPaused))
        );
        assert_eq!(client.provider_stake(&provider), 1_000);
        assert_eq!(client.total_stake(), 1_000);

        // Withdrawals remain available subject to normal balance checks.
        client.withdraw(&provider, &400);
        assert_eq!(client.provider_stake(&provider), 600);

        // Unpausing restores delegations.
        client.set_paused(&false);
        let pause_off_events = env.events().all();
        client.delegate(&provider, &400);
        assert_eq!(client.provider_stake(&provider), 1_000);

        // Both toggles emitted an auditable event attributed to the admin.
        assert!(pause_on_events.iter().any(|(_, _, data)| {
            PauseChanged::try_from_val(&env, &data).map_or(false, |e| {
                e.paused
                    && e.actor == admin.clone()
            })
        }));
        assert!(pause_off_events.iter().any(|(_, _, data)| {
            PauseChanged::try_from_val(&env, &data).map_or(false, |e| {
                !e.paused
                    && e.actor == admin.clone()
            })
        }));
    }

    #[test]
    fn unauthorized_pause_toggle_is_rejected() {
        // Issue #1266: only the administrator may change the pause state.
        let env = Env::default();
        // No `mock_all_auths()`: the admin authorization must be supplied.
        let contract_id = env.register_contract(None, StakeVault);
        let client = StakeVaultClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin, &10_000);

        assert!(client.try_set_paused(&true).is_err());
        assert!(!client.is_paused());
    }

    // Reference model for issue #1267: the concentration policy computed in
    // plain integers, independent of the contract implementation.
    fn model_within(provider_stake: i128, total_stake: i128, max_bps: u32) -> bool {
        if total_stake <= 0 {
            return true;
        }
        provider_stake * 10_000 <= total_stake * (max_bps as i128)
    }

    #[test]
    fn model_matches_chain_across_interleaved_operations() {
        // Issue #1267: compare the vault against a reference model as
        // delegations, withdrawals, and limit changes are interleaved.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, StakeVault);
        let client = StakeVaultClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin, &10_000);

        let p0 = Address::generate(&env);
        let p1 = Address::generate(&env);
        let providers = [p0.clone(), p1.clone()];

        let mut model = [0i128, 0i128];
        let mut total = 0i128;
        let mut bps: u32 = 10_000;

        // Deterministic sequence: 0 = delegate, 1 = withdraw, 2 = set limit.
        let ops: [(u8, usize, i128, u32); 12] = [
            (0, 0, 500, 0),
            (0, 1, 300, 0),
            (1, 0, 100, 0),
            (2, 0, 0, 5_000),
            (0, 0, 500, 0),
            (0, 1, 100, 0),
            (1, 1, 50, 0),
            (2, 0, 0, 10_000),
            (0, 1, 1_000, 0),
            (1, 0, 900, 0),
            (0, 0, 1, 0),
            (1, 1, 1_000, 0),
        ];

        for (op, idx, amount, new_bps) in ops {
            match op {
                0 => {
                    let expected_ok =
                        amount > 0 && model_within(model[idx] + amount, total + amount, bps);
                    let res = client.try_delegate(&providers[idx], &amount);
                    if expected_ok {
                        assert!(res.is_ok(), "model expected delegation to succeed");
                        model[idx] += amount;
                        total += amount;
                    } else {
                        assert!(res.is_err(), "model expected delegation to be rejected");
                    }
                }
                1 => {
                    let expected_ok = amount > 0 && amount <= model[idx];
                    let res = client.try_withdraw(&providers[idx], &amount);
                    if expected_ok {
                        assert!(res.is_ok(), "model expected withdrawal to succeed");
                        model[idx] -= amount;
                        total -= amount;
                    } else {
                        assert!(res.is_err(), "model expected withdrawal to be rejected");
                    }
                }
                _ => {
                    client.set_max_provider_share_bps(&new_bps);
                    bps = new_bps;
                }
            }

            // After every operation the on-chain state must match the model.
            assert_eq!(client.total_stake(), total);
            for (i, p) in providers.iter().enumerate() {
                assert_eq!(client.provider_stake(p), model[i]);
            }
        }
    }

    #[test]
    fn withdraw_requires_provider_authorization() {
        // Issue #1258: without the provider's authorization the withdrawal is
        // rejected and neither provider nor total stake changes.
        let env = Env::default();
        // Intentionally no `mock_all_auths()`: the withdraw must carry a real auth.
        let contract_id = env.register_contract(None, StakeVault);
        let client = StakeVaultClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);
        client.initialize(&admin, &10_000);
        client.delegate(&provider, &500);

        assert!(client.try_withdraw(&provider, &100).is_err());
        assert_eq!(client.provider_stake(&provider), 500);
        assert_eq!(client.total_stake(), 500);
    }

    #[test]
    fn authorized_partial_and_full_withdrawals_succeed() {
        // Issue #1258: an authorized provider can withdraw partially and fully.
        let (_env, client, _admin, provider) = setup(10_000);
        client.delegate(&provider, &1_000);
        client.withdraw(&provider, &400);
        assert_eq!(client.provider_stake(&provider), 600);
        client.withdraw(&provider, &600);
        assert_eq!(client.provider_stake(&provider), 0);
        assert_eq!(client.total_stake(), 0);
    }

    #[test]
    fn delegate_multiplication_overflow_is_rejected_without_state_change() {
        // Issue #1261: a stake whose basis-point comparison is unrepresentable
        // fails with a stable error and leaves accounting untouched.
        let (_env, client, _admin, provider) = setup(10_000);
        let huge = i128::MAX / 10_000 + 1;
        assert_eq!(
            client.try_delegate(&provider, &huge),
            Err(Ok(VaultError::ArithmeticOverflow))
        );
        assert_eq!(client.provider_stake(&provider), 0);
        assert_eq!(client.total_stake(), 0);
    }

    #[test]
    fn delegate_addition_overflow_is_rejected_without_state_change() {
        // Issue #1261: a stake that pushes the running total past i128 fails
        // before any persistent write.
        let (_env, client, _admin, provider) = setup(10_000);
        let safe = i128::MAX / 10_001;
        client.delegate(&provider, &safe);
        assert_eq!(client.provider_stake(&provider), safe);
        assert_eq!(client.total_stake(), safe);

        assert_eq!(
            client.try_delegate(&provider, &(i128::MAX - 10)),
            Err(Ok(VaultError::ArithmeticOverflow))
        );
        assert_eq!(client.provider_stake(&provider), safe);
        assert_eq!(client.total_stake(), safe);
    }

    #[test]
    fn full_withdrawal_removes_provider_entry() {
        // Issue #1263: a full withdrawal must drop the persistent entry, while
        // a partial withdrawal keeps it.
        let (env, client, _admin, provider) = setup(10_000);
        client.delegate(&provider, &500);

        // Partial withdrawal retains the entry.
        client.withdraw(&provider, &200);
        assert_eq!(client.provider_stake(&provider), 300);
        env.as_contract(&client.address, || {
            assert!(env
                .storage()
                .persistent()
                .has(&DataKey::ProviderStake(provider.clone())));
        });

        // Full withdrawal removes the entry; queries still report zero.
        client.withdraw(&provider, &300);
        assert_eq!(client.provider_stake(&provider), 0);
        assert_eq!(client.total_stake(), 0);
        env.as_contract(&client.address, || {
            assert!(!env
                .storage()
                .persistent()
                .has(&DataKey::ProviderStake(provider.clone())));
        });
    }

    #[test]
    fn delegation_and_withdrawal_emit_auditable_events() {
        // Issue #1262: successful stake changes emit a structured event with
        // the operation, provider, amount, and resulting balances.
        let (env, client, _admin, provider) = setup(10_000);
        client.delegate(&provider, &500);

        let events = env.events().all();
        let delegated = StakeChangedEvent {
            operation: Symbol::new(&env, "stake_delegated"),
            provider: provider.clone(),
            amount: 500,
            provider_balance: 500,
            total_balance: 500,
        };
        assert!(
            events.iter().any(|(_, _, data)| {
                StakeChangedEvent::try_from_val(&env, &data).map_or(false, |e| e == delegated)
            }),
            "expected a stake_delegated event describing the delegation"
        );

        client.withdraw(&provider, &500);
        let events = env.events().all();
        let withdrawn = StakeChangedEvent {
            operation: Symbol::new(&env, "stake_withdrawn"),
            provider: provider.clone(),
            amount: 500,
            provider_balance: 0,
            total_balance: 0,
        };
        assert!(
            events.iter().any(|(_, _, data)| {
                StakeChangedEvent::try_from_val(&env, &data).map_or(false, |e| e == withdrawn)
            }),
            "expected a stake_withdrawn event describing the withdrawal"
        );
    }

    #[test]
    fn rejected_operations_emit_no_stake_event() {
        // Issue #1262: failed operations must not emit a success event.
        let (env, client, _admin, provider) = setup(10_000);
        client.delegate(&provider, &500);
        // Tighten the limit so the next delegation is rejected.
        client.set_max_provider_share_bps(&5_000);

        let before = env.events().all().len();
        assert!(client.try_delegate(&provider, &1).is_err());
        assert_eq!(
            env.events().all().len(),
            before,
            "rejected delegation must not emit a stake event"
        );
    }
    }
}
