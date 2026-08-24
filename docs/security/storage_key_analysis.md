# Storage Key Collision Analysis

**Issue:** #265  
**Status:** No collision found  
**Last updated:** 2026-04-25

---

## Why Collisions Cannot Occur

All storage keys in this codebase are defined with the `#[contracttype]` macro on Rust enums.
Soroban serialises a `#[contracttype]` enum value as an XDR `ScVal::Map` containing the variant
name as a symbol key. Because the variant name is always part of the serialised bytes, two
variants with the same payload but different names produce different byte sequences.

For tuple variants that accept an `Address` (or other data), the serialised form is:

```
{ "<VariantName>": <payload> }
```

Two addresses that differ in even one byte produce a different `<payload>`, so
`Variant(addr_a) ≠ Variant(addr_b)` whenever `addr_a ≠ addr_b`.

Cross-enum collision is impossible because each enum is a distinct Rust type; Soroban
encodes the type name as part of the outer map key, so `EnumA::Foo` and `EnumB::Foo`
serialise differently.

---

## Key Inventory by Contract

### 1. `fee_collector` — `StorageKey`

File: `contracts/fee_collector/src/storage.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Admin` | — | instance |
| `Initialized` | — | instance |
| `OracleContract` | — | instance |
| `FeeRate` | — | instance |
| `BurnRate` | — | instance |
| `QueuedWithdrawal` | — | instance |
| `TreasuryBalance(Address)` | token address | persistent |
| `ProviderPendingFees(Address, Address)` | (provider, token) | persistent |
| `MonthlyTradeVolume(Address)` | user address | persistent |

No two variants share the same name. The two-address tuple `ProviderPendingFees(p, t)` is
distinct from `TreasuryBalance(t)` because the variant names differ and the tuple arity
differs.

---

### 2. `user_portfolio` — `DataKey`

File: `contracts/user_portfolio/src/storage.rs`  
Extended by: `badges.rs`, `subscriptions.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Initialized` | — | instance |
| `Admin` | — | instance |
| `Oracle` | — | instance |
| `OracleAssetPair` | — | instance |
| `NextPositionId` | — | instance |
| `TradeExecutor` | — | instance |
| `EarlyAdopterCap` | — | instance |
| `TotalUsersFirstOpen` | — | instance |
| `Position(u64)` | position id | persistent |
| `UserPositions(Address)` | user address | persistent |
| `UserBadges(Address)` | user address | persistent |
| `UserClosedTradeCount(Address)` | user address | persistent |
| `UserProfitStreak(Address)` | user address | persistent |
| `LeaderboardRank(Address)` | user address | persistent |

`UserPositions`, `UserBadges`, `UserClosedTradeCount`, `UserProfitStreak`, and
`LeaderboardRank` all take an `Address` but have distinct variant names — no collision.

**`user_portfolio` — `StorageKey`** (subscriptions module)

| Variant | Payload | Storage tier |
|---|---|---|
| `Subscription(Address, Address)` | (user, provider) | persistent |
| `ProviderTerms(Address)` | provider address | persistent |

`Subscription(user, provider)` and `Subscription(provider, user)` are different values
because the tuple is ordered. A user cannot accidentally collide with a provider's terms
entry because the variant names differ.

---

### 3. `oracle` — `StorageKey` (storage.rs)

File: `contracts/oracle/src/storage.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `BaseCurrency` | — | persistent |
| `Price(AssetPair)` | asset pair | persistent |
| `PriceTimestamp(AssetPair)` | asset pair | persistent |
| `AvailablePairs` | — | persistent |
| `ConversionCache(Asset, Asset)` | (from, to) assets | temporary |

`Price(pair)` and `PriceTimestamp(pair)` share the same payload but have different variant
names — no collision.

**`oracle` — `StorageKey`** (types.rs — consensus/reputation layer)

File: `contracts/oracle/src/types.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Admin` | — | instance |
| `Guardian` | — | instance |
| `PriceMap(AssetPair)` | asset pair | persistent |
| `OracleStats` | — | instance |
| `Oracles` | — | instance |
| `PriceSubmissions` | — | instance |
| `ConsensusPrice` | — | instance |
| `PauseStates` | — | instance |
| `OracleWeight(Address)` | oracle address | persistent |
| `PendingAdmin` | — | instance |
| `PendingAdminExpiry` | — | instance |

These two `StorageKey` enums live in different modules (`storage.rs` vs `types.rs`) and are
used in different call sites. They are distinct Rust types; Soroban encodes the type name,
so `storage::StorageKey::Price(p)` and `types::StorageKey::PriceMap(p)` cannot collide.

---

### 4. `auto_trade` — `DataKey`

File: `contracts/auto_trade/src/storage.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Trades(Address, u64)` | (user, trade_id) | persistent |
| `Signal(u64)` | signal id | persistent |

**`auto_trade` — `AdminStorageKey`**

File: `contracts/auto_trade/src/admin.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Admin` | — | instance |
| `Guardian` | — | instance |
| `OracleAddress` | — | instance |
| `OracleCircuitBreaker` | — | instance |
| `OracleWhitelist(u32)` | asset_pair id | instance |
| `PauseStates` | — | instance |
| `CircuitBreakerStats` | — | instance |
| `CircuitBreakerConfig` | — | instance |
| `PendingAdmin` | — | instance |
| `PendingAdminExpiry` | — | instance |

**`auto_trade` — `AuthKey`**

File: `contracts/auto_trade/src/auth.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Authorization(Address)` | user address | persistent |

**`auto_trade` — `PositionKey`**

File: `contracts/auto_trade/src/positions.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Position(BytesN<32>)` | trade_id hash | persistent |
| `UserPositions(Address)` | user address | persistent |
| `PositionNonce` | — | persistent |

**`auto_trade` — `ExitStrategyKey`**

File: `contracts/auto_trade/src/exit_strategy.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Strategy(u64)` | strategy_id | persistent |
| `NextId` | — | persistent |
| `UserStrategies(Address)` | user address | persistent |

**`auto_trade` — `HistoryDataKey`**

File: `contracts/auto_trade/src/history.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `UserTradeCount(Address)` | user address | persistent |
| `Trade(Address, u64)` | (user, trade_index) | persistent |

`UserTradeCount(addr_a)` and `UserTradeCount(addr_b)` are distinct. `Trade(addr_a, n)` and
`Trade(addr_b, n)` are distinct because the address bytes differ.

**`auto_trade` — `ReferralKey`**

File: `contracts/auto_trade/src/referral.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Entry(Address)` | referee address | persistent |
| `Stats(Address)` | referrer address | persistent |
| `Referees(Address)` | referrer address | persistent |

`Entry`, `Stats`, and `Referees` have distinct variant names — no collision even when the
same address is used as both referee and referrer.

**`auto_trade` — `RiskDataKey`**

File: `contracts/auto_trade/src/risk.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `UserRiskConfig(Address)` | user address | persistent |
| `UserRiskParityConfig(Address)` | user address | persistent |
| `UserPositions(Address)` | user address | persistent |
| `UserTradeHistory(Address)` | user address | persistent |
| `AssetPrice(u32)` | asset_id | persistent |
| `AssetPriceHistory(u32, u32)` | (asset_id, slot) | persistent |
| `AssetPriceHistoryCount(u32)` | asset_id | persistent |

All four user-address variants have distinct names — no collision.

**`auto_trade` — `InsuranceKey`**

File: `contracts/auto_trade/src/portfolio_insurance.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Insurance(Address)` | user address | persistent |

**`auto_trade` — `PairsDataKey`**

File: `contracts/auto_trade/src/strategies/pairs_trading.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Strategy(Address, u64)` | (user, strategy_id) | persistent |
| `NextStrategyId` | — | persistent |
| `NextPositionId` | — | persistent |

**`auto_trade` — `StatArbDataKey`**

File: `contracts/auto_trade/src/strategies/stat_arb.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Strategy(Address)` | user address | persistent |
| `ActivePortfolio(Address)` | user address | persistent |
| `PriceHistory(u32)` | asset_id | persistent |
| `NextPortfolioId` | — | persistent |

`Strategy` and `ActivePortfolio` have distinct variant names — no collision.

**`auto_trade` — `RateLimitKey`**

File: `contracts/auto_trade/src/rate_limit.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Limits` | — | persistent |
| `UserHistory(Address)` | user address | persistent |
| `Whitelist` | — | persistent |
| `GlobalHourlyCount` | — | persistent |
| `Admin` | — | persistent |

**`auto_trade` — other strategy keys** (`GridDataKey`, `MomentumDataKey`, `BreakoutDataKey`, `DCAKey`, `ConditionalKey`, `IcebergStorageKey`, `CorrKey`, `TWAPStorageKey`, `EmergencyKey`, `MLDataKey`, `MRKey`, `SentimentStorageKey`, `ArbStorageKey`, `SmartRoutingKey`)

All variants in these enums are keyed by `u64`, `u32`, or `AssetPair` — not by `Address`.
No user-keyed collision risk.

`DataKey`, `AdminStorageKey`, `AuthKey`, and all strategy-specific key enums are distinct
Rust types — no cross-enum collision possible.

---

### 5. `signal_registry` — `StorageKey`

File: `contracts/signal_registry/src/lib.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `SignalCounter` | — | instance |
| `Signals` | — | persistent (map) |
| `SignalsV1` | — | persistent (map) |
| `MigrationCursor` | — | instance |
| `MigrationV1TargetTotal` | — | instance |
| `ProviderStats` | — | persistent |
| `ProviderStakes` | — | persistent |
| `TradeExecutions` | — | persistent |
| `TradeCounter` | — | instance |
| `TemplateCounter` | — | instance |
| `Templates` | — | persistent |
| `ExternalIdMappings` | — | persistent |
| `ComboCounter` | — | instance |
| `Combos` | — | persistent |
| `ComboExecutions(u64)` | combo id | persistent |
| `CrossChainSignals(String, String)` | (chain, signal_id) | persistent |
| `AddressMappings(String, String)` | (chain, address) | persistent |
| `ActiveSignalsByCategory` | — | persistent |
| `AdoptionNonces` | — | persistent |
| `TradeExecutor` | — | instance |
| `UserPortfolio` | — | instance |
| `RecordedSignalOutcomes` | — | persistent |
| `ProviderReputationScore(Address)` | provider address | persistent |

**`signal_registry` — `AdminStorageKey`**

File: `contracts/signal_registry/src/admin.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Admin` | — | instance |
| `PendingAdminTransfer` | — | instance |
| `Guardian` | — | instance |
| `MinStake` | — | instance |
| `TradeFee` | — | instance |
| `StopLoss` | — | instance |
| `PositionLimit` | — | instance |
| `PauseStates` | — | instance |
| `CircuitBreakerStats` | — | instance |
| `CircuitBreakerConfig` | — | instance |
| `MultiSigEnabled` | — | instance |
| `MultiSigSigners` | — | instance |
| `MultiSigThreshold` | — | instance |
| `FeeCollectionPaused` | — | instance |
| `PendingAdmin` | — | instance |
| `PendingAdminExpiry` | — | instance |

**`signal_registry` — `FeeStorageKey`**

File: `contracts/signal_registry/src/types.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `PlatformTreasury` | — | persistent |
| `ProviderTreasury` | — | persistent |
| `TreasuryBalances` | — | persistent |

**`signal_registry` — `SocialDataKey`**

File: `contracts/signal_registry/src/social.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Follow(Address, Address)` | (user, provider) | instance |
| `UserFollowedList(Address)` | user address | instance |
| `FollowerCount(Address)` | provider address | instance |

`Follow(user, provider)` and `Follow(provider, user)` are distinct because the tuple is
ordered. `UserFollowedList` and `FollowerCount` have different variant names.

**`signal_registry` — `ReputationDataKey`**

File: `contracts/signal_registry/src/reputation.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `TrustScore(Address)` | provider address | persistent |
| `FirstSignalTime(Address)` | provider address | persistent |
| `MedianStake` | — | persistent |
| `MedianFollowers` | — | persistent |

`TrustScore` and `FirstSignalTime` share the same payload type but have distinct variant
names — no collision.

**`signal_registry` — `VersioningStorageKey`**

File: `contracts/signal_registry/src/versioning.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `SignalVersions(u64, u32)` | (signal_id, version) | persistent |
| `LatestVersion(u64)` | signal_id | persistent |
| `UpdateCount(u64)` | signal_id | persistent |
| `LastUpdateTime(u64)` | signal_id | persistent |
| `CopyRecords(Address, u64)` | (user, signal_id) | persistent |

`CopyRecords(user_a, id)` and `CopyRecords(user_b, id)` are distinct because the address
bytes differ.

**`signal_registry` — `ScheduleDataKey`**

File: `contracts/signal_registry/src/scheduling.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Schedule(u64)` | schedule_id | persistent |
| `ProviderSchedules(Address)` | provider address | persistent |
| `NextScheduleId` | — | persistent |

**`signal_registry` — `SizingDataKey`**

File: `contracts/signal_registry/src/position_sizing.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `UserSizingConfig(Address)` | user address | persistent |
| `PriceHistory(u32, u32)` | (asset_id, slot) | persistent |
| `PriceHistoryLen(u32)` | asset_id | persistent |
| `PriceHistoryHead(u32)` | asset_id | persistent |

**`signal_registry` — `LeaderboardKey`**, **`TagStorageKey`**, **`CollabStorageKey`**, **`MLStorageKey`**, **`ContestStorageKey`**

Files: `leaderboard.rs`, `categories.rs`, `collaboration.rs`, `ml_scoring.rs`, `contests.rs`

All variants in these enums are unit variants or keyed by `u64` (not `Address`). No
user-keyed collision risk.

---

### 6. `trade_executor` — `StorageKey`

File: `contracts/trade_executor/src/lib.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Admin` | — | instance |
| `UserPortfolio` | — | instance |
| `Oracle` | — | instance |
| `StopLossPortfolio` | — | instance |
| `CopyTradeEstimatedFee` | — | instance |
| `SdexRouter` | — | instance |
| `PositionLimitExempt(Address)` | user address | instance |
| `LastInsufficientBalance(Address)` | user address | instance |

Stop-loss and take-profit prices use raw tuple keys (not a `#[contracttype]` enum):

```rust
// triggers.rs
(Symbol::new(env, "StopLoss"),  user, trade_id)  // persistent
(Symbol::new(env, "TakeProfit"), user, trade_id) // persistent
```

These are `(Symbol, Address, u64)` tuples. The `Symbol` discriminant (`"StopLoss"` vs
`"TakeProfit"`) ensures the two cannot collide with each other. They also cannot collide
with `StorageKey` variants because the outer type differs.

The reentrancy lock uses a plain `Symbol`:

```rust
Symbol::new(env, "ExecLock")  // temporary storage
```

This is in temporary storage and is a different type from all persistent/instance keys.

---

### 7. `governance` — `StorageKey`

File: `contracts/governance/src/lib.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Admin` | — | instance |
| `Initialized` | — | instance |
| `Metadata` | — | instance |
| `Balances` | — | instance |
| `StakedBalances` | — | instance |
| `PendingRewards` | — | instance |
| `VestingSchedules` | — | instance |
| `Holders` | — | instance |
| `DistributionState` | — | instance |
| `VoteLocks` | — | instance |
| `Treasury` | — | instance |
| `Committees` | — | instance |
| `GovernanceConfig` | — | instance |
| `ProposalsState` | — | instance |
| `Delegations` | — | instance |
| `TimelockState` | — | instance |
| `Guardian` | — | instance |
| `GovernanceParameters` | — | instance |
| `GovernanceFeatures` | — | instance |
| `GovernanceUpgrades` | — | instance |
| `ReputationState` | — | instance |
| `VoteRecords` | — | instance |
| `ConvictionState` | — | instance |
| `ContractPaused` | — | instance |

Governance uses no user-keyed tuple variants in its primary `StorageKey`. Per-user data
(balances, votes, credits) is stored inside map values keyed by `Address` within a single
top-level entry (e.g., `StorageKey::Balances` holds a `Map<Address, i128>`). This design
means there are no user-keyed storage entries that could collide.

---

### 8. `stake_vault` — `StorageKey`

File: `contracts/stake_vault/src/lib.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Admin` | — | instance |
| `StakeToken` | — | instance |

Both variants are unit variants (no payload). They have distinct names so cannot collide
with each other. The reentrancy lock uses a plain `Symbol::new(&env, "WithdrawLock")` in
**temporary** storage — a different storage tier and a different key type from all
instance/persistent keys.

**`stake_vault` — `MigrationKey`**

File: `contracts/stake_vault/src/migration.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `StakesV1` | — | persistent |
| `StakesV2` | — | persistent |
| `MigrationState` | — | persistent |

All three are unit variants with distinct names. `StakesV1` and `StakesV2` hold
`Map<Address, _>` values — the per-staker lookup is done inside the map value, not as
separate storage keys, so there are no user-keyed storage entries in `stake_vault` that
could collide across users.

`MigrationKey` is a distinct Rust type from `StorageKey`; Soroban encodes the type name,
so `MigrationKey::StakesV2` cannot collide with `StorageKey::Admin` or any variant in any
other contract's key enum.

---

### 9. `bridge` — `DataKey`

File: `contracts/bridge/src/lib.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Config` | — | persistent |
| `WrappedAsset(String)` | asset symbol | persistent |
| `Transfer(u64)` | transfer_id | persistent |
| `ReplayLock(ChainId, String, u64)` | (chain, tx_hash, nonce) | persistent |
| `UsedSignature(Address, u64, ValidatorApprovalKind, String)` | (validator, nonce, kind, hash) | persistent |
| `WrappedBalance(Address, String)` | (user, asset_symbol) | persistent |
| `DailyVolume` | — | persistent |

`UsedSignature(addr_a, ...)` and `UsedSignature(addr_b, ...)` are distinct because the
address bytes differ. `WrappedBalance(user_a, sym)` and `WrappedBalance(user_b, sym)` are
distinct for the same reason.

**`bridge` — other key enums** (`FeeStorageKey`, `MonitoringDataKey`, `MessagingKey`, `LiquidityKey`, `AnalyticsDataKey`, `GovernanceDataKey`)

None of these contain `Address`-parameterised variants. No user-keyed collision risk.

---

### 10. `common` — `ReplayKey`

File: `contracts/common/src/replay_protection.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `UserNonce(Address)` | user address | persistent |
| `TxHash(Bytes)` | tx hash bytes | persistent |

`UserNonce` and `TxHash` have distinct variant names. `UserNonce(addr_a)` and
`UserNonce(addr_b)` are distinct because the address bytes differ.

**`common` — `RateLimitKey`**

File: `contracts/common/src/rate_limit.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Timestamps(Address, ActionType)` | (user, action) | persistent |
| `Config(ActionType)` | action type | persistent |
| `UserFirstAction(Address)` | user address | persistent |

`Timestamps(addr_a, action)` and `Timestamps(addr_b, action)` are distinct. `Timestamps`
and `UserFirstAction` have distinct variant names — no collision even for the same address.

---

### 11. `oracle` — `GovernanceKey`

File: `contracts/oracle/src/governance.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `ProposalCounter` | — | persistent |
| `Proposal(u64)` | proposal_id | persistent |
| `HasVoted(u64, Address)` | (proposal_id, voter) | persistent |
| `TotalStaked` | — | persistent |
| `Stake(Address)` | staker address | persistent |
| `GovAdmin` | — | persistent |

`HasVoted(id, addr_a)` and `HasVoted(id, addr_b)` are distinct. `Stake(addr_a)` and
`Stake(addr_b)` are distinct. `HasVoted` and `Stake` have distinct variant names — no
collision even when the same address is used.

---

### 12. `governance` — `QVStorageKey`

File: `contracts/governance/src/quadratic_voting.rs`

| Variant | Payload | Storage tier |
|---|---|---|
| `Config` | — | persistent |
| `Credits(Address)` | user address | persistent |
| `Vote(u64, Address)` | (proposal_id, voter) | persistent |
| `ProposalVoters(u64)` | proposal_id | persistent |
| `Identity(Address)` | user address | persistent |

`Credits(addr_a)` and `Credits(addr_b)` are distinct. `Vote(id, addr_a)` and
`Vote(id, addr_b)` are distinct. `Credits`, `Vote`, and `Identity` have distinct variant
names — no collision even for the same address.

---

## Collision Risk Assessment

### Same-enum, same-variant, different payload

For any variant `V(addr)`, two different addresses `addr_a ≠ addr_b` produce different
serialised keys because the address bytes are part of the XDR payload. This is the
primary concern raised in the issue.

**Verdict: No collision possible.** Soroban address serialisation is injective.

### Same-enum, different-variant, same payload

Example: `oracle::storage::StorageKey::Price(pair)` vs
`oracle::storage::StorageKey::PriceTimestamp(pair)`.

The variant name is encoded as an XDR symbol in the map key, so different variant names
always produce different byte sequences regardless of payload.

**Verdict: No collision possible.**

### Cross-enum, same variant name, same payload

Example: `fee_collector::StorageKey::Admin` vs `governance::StorageKey::Admin`.

Each `#[contracttype]` enum is a distinct Rust type. Soroban encodes the type name as part
of the outer XDR structure. Two enums with the same variant name but different type names
serialise differently.

Furthermore, each contract has its own isolated storage namespace — storage written by
`fee_collector` is never readable by `governance` and vice versa.

**Verdict: No collision possible.**

### Tuple-variant ordering

`ProviderPendingFees(provider, token)` — if the arguments were swapped, a provider address
used as a token address would collide. However, the call sites always pass `(provider,
token)` in the documented order, and the types are both `Address` so the compiler does not
catch a swap. This is a **code-review concern**, not a serialisation collision.

`Subscription(user, provider)` — same concern. The call sites consistently pass
`(user, provider)`.

**Verdict: No serialisation collision. Argument-order discipline is enforced by code
review and the accessor functions in `storage.rs` / `subscriptions.rs`.**

---

## Summary

| Risk | Verdict |
|---|---|
| Same variant, different `Address` payload | **No collision** — address bytes are injective |
| Different variant, same payload | **No collision** — variant name is in serialised bytes |
| Cross-enum, same variant name | **No collision** — type name differs; contracts have isolated storage |
| Tuple argument order swap | **Not a serialisation collision** — code-review concern only |

No storage key collision is possible in the current codebase.

---

## Tests

See `contracts/common/src/storage_key_tests.rs`:

| Test | What it verifies |
|---|---|
| `single_addr_variants_differ_for_different_users` | All single-`Address` variants produce different keys for different users |
| `two_addr_variants_differ_for_different_users` | Two-`Address` variants differ when the first argument changes |
| `two_addr_variant_argument_order_matters` | `Subscription(a, b) ≠ Subscription(b, a)` |
| `different_variants_same_address_do_not_collide` | Different variant names with the same address payload are distinct |
| `stake_vault_migration_keys_are_distinct` | `MigrationKey::StakesV1`, `StakesV2`, and `MigrationState` are all distinct |
| `stake_vault_storage_keys_are_distinct` | `StorageKey::Admin` and `StorageKey::StakeToken` are distinct |
| `signal_registry_social_keys_differ_for_different_users` | `SocialDataKey` user-keyed variants are distinct across users |
| `signal_registry_reputation_keys_differ_for_different_providers` | `ReputationDataKey` variants are distinct across providers |
| `signal_registry_versioning_copy_records_differ_for_different_users` | `VersioningStorageKey::CopyRecords(user, id)` is distinct across users |
| `auto_trade_user_keyed_variants_differ_for_different_users` | `PositionKey`, `ExitStrategyKey`, `HistoryDataKey`, `ReferralKey`, `RiskDataKey`, `InsuranceKey` are distinct across users |
| `auto_trade_pairs_strategy_differs_for_different_users` | `PairsDataKey::Strategy(user, id)` is distinct across users |
| `auto_trade_stat_arb_keys_differ_for_different_users` | `StatArbDataKey` user-keyed variants are distinct across users |
| `bridge_user_keyed_variants_differ_for_different_users` | `bridge::DataKey::WrappedBalance(user, sym)` is distinct across users |
| `common_replay_key_differs_for_different_users` | `ReplayKey::UserNonce(addr)` is distinct across users |
| `common_rate_limit_key_differs_for_different_users` | `RateLimitKey::Timestamps` and `UserFirstAction` are distinct across users |
| `oracle_governance_key_differs_for_different_users` | `GovernanceKey::Stake` and `HasVoted` are distinct across users |
| `governance_qv_keys_differ_for_different_users` | `QVStorageKey::Credits`, `Vote`, `Identity` are distinct across users |
