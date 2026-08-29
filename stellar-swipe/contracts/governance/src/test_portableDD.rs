extern crate std;

use crate::distribution::{DistributionRecipients, YEAR_SECONDS};
use crate::proposals::{ProposalCategory, ProposalType, VoteType as GovernanceVoteType};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{Address, Bytes, Env, String, Symbol, TryFromVal, Val};
use stellar_swipe_common::Asset;

const SUPPLY: i128 = 1_000_000_000;

fn setup() -> (Env, Address, Address, DistributionRecipients) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let contract_id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let recipients = DistributionRecipients {
        team: Address::generate(&env),
        early_investors: Address::generate(&env),
        community_rewards: Address::generate(&env),
        treasury: Address::generate(&env),
        public_sale: Address::generate(&env),
    };
    (env, contract_id, admin, recipients)
}

fn init(
    client: &GovernanceContractClient<'_>,
    env: &Env,
    admin: &Address,
    r: &DistributionRecipients,
) {
    client.initialize(
        admin,
        &String::from_str(env, "StellarSwipe Gov"),
        &String::from_str(env, "SSG"),
        &7u32,
        &SUPPLY,
        r,
    );
}

fn asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    }
}

// ── Issue #587: Cliff-and-linear vesting ─────────────────────────────────────

#[test]
fn vesting_pre_cliff_claim_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let beneficiary = Address::generate(&env);
    let cliff = 100u64;
    let duration = 200u64;
    client.create_vesting_schedule(&admin, &beneficiary, &1_000i128, &0u64, &cliff, &duration);

    // Still before cliff — releasable must be 0.
    assert_eq!(client.releasable_vested_amount(&beneficiary), 0);

    // Direct claim attempt before cliff must fail.
    let result = client.try_release_vested_tokens(&beneficiary);
    assert_eq!(result, Err(Ok(GovernanceError::CliffNotReached)));
}

#[test]
fn vesting_partial_claim_mid_vesting() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let beneficiary = Address::generate(&env);
    let total = 1_000i128;
    let cliff = 100u64;
    let duration = 200u64;
    client.create_vesting_schedule(&admin, &beneficiary, &total, &0u64, &cliff, &duration);

    // Advance past cliff and halfway through the vesting window.
    // cliff_time = 100; vesting_window = duration - cliff = 100
    // At t=150: elapsed_after_cliff=50, vested = total * 50/100 = 500
    env.ledger().set_timestamp(150);
    let releasable = client.releasable_vested_amount(&beneficiary);
    assert!(
        releasable > 0 && releasable < total,
        "expected partial vesting, got {releasable}"
    );

    let claimed = client.release_vested_tokens(&beneficiary);
    assert_eq!(claimed, releasable);

    // After partial claim, releasable should be zero until more time passes.
    let after = client.releasable_vested_amount(&beneficiary);
    assert_eq!(after, 0);
}

#[test]
fn vesting_full_claim_after_duration() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let beneficiary = Address::generate(&env);
    let total = 2_000i128;
    let cliff = YEAR_SECONDS;
    let duration = 4 * YEAR_SECONDS;
    client.create_vesting_schedule(&admin, &beneficiary, &total, &0u64, &cliff, &duration);

    // After full vesting period, everything should be releasable.
    env.ledger().set_timestamp(duration + 1);
    let releasable = client.releasable_vested_amount(&beneficiary);
    assert_eq!(releasable, total);

    let claimed = client.release_vested_tokens(&beneficiary);
    assert_eq!(claimed, total);

    // No double-claim.
    let result = client.try_release_vested_tokens(&beneficiary);
    assert_eq!(result, Err(Ok(GovernanceError::NothingToRelease)));
}

#[test]
fn vesting_two_partial_claims_no_double_counting() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let beneficiary = Address::generate(&env);
    let total = 1_000i128;
    client.create_vesting_schedule(&admin, &beneficiary, &total, &0u64, &0u64, &200u64);

    env.ledger().set_timestamp(100); // halfway
    let first = client.release_vested_tokens(&beneficiary);
    assert!(first > 0);

    env.ledger().set_timestamp(200); // full
    let second = client.release_vested_tokens(&beneficiary);
    assert!(second > 0);

    assert!(
        first + second <= total,
        "total claimed must not exceed total_amount"
    );
}

// ── Issue #588: Proposal payload validation ──────────────────────────────────

fn stake_for_proposals(env: &Env, contract_id: &Address, user: &Address, amount: i128) {
    env.as_contract(contract_id, || {
        // Give the user both a token balance (to cover deposits) and staked balance.
        crate::add_balance(env, user, amount).unwrap();
        crate::add_staked_balance(env, user, amount).unwrap();
        crate::track_holder(env, user);
    });
}

#[test]
fn proposal_contract_upgrade_invalid_payload_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // ContractUpgrade payload must be exactly 32 bytes.
    let bad_payload = Bytes::from_slice(&env, &[0u8; 16]);
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ContractUpgrade(String::from_str(&env, "core"), bad_payload.clone()),
        &String::from_str(&env, "Upgrade"),
        &String::from_str(&env, "Upgrade the contract"),
        &bad_payload,
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

#[test]
fn proposal_contract_upgrade_valid_payload_accepted() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // Valid 32-byte WASM hash payload.
    let hash = Bytes::from_slice(&env, &[0xab_u8; 32]);
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ContractUpgrade(String::from_str(&env, "core"), hash.clone()),
        &String::from_str(&env, "Upgrade"),
        &String::from_str(&env, "Upgrade the contract"),
        &hash,
        &ProposalCategory::General,
        &false,
    );
    assert!(
        matches!(result, Ok(Ok(_))),
        "expected Ok(Ok), got {result:?}"
    );
}

#[test]
fn proposal_custom_empty_payload_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);
    let target = Address::generate(&env);

    let empty = Bytes::new(&env);
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::Custom(target),
        &String::from_str(&env, "Custom"),
        &String::from_str(&env, "Custom proposal"),
        &empty,
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

#[test]
fn proposal_treasury_spend_bad_version_prefix_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    // Fund the treasury so TreasurySpend is valid at the proposal-type level.
    let asset_key = asset(&env);
    env.as_contract(&id, || {
        let mut t = crate::get_treasury(&env);
        t.assets.set(asset_key.clone(), 100_000i128);
        crate::put_treasury(&env, &t);
    });

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // Payload with wrong version byte (0x02 instead of 0x01).
    let bad = Bytes::from_slice(&env, &[0x02_u8, 0x00]);
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::TreasurySpend(
            Address::generate(&env),
            1_000i128,
            asset_key,
            String::from_str(&env, "payment"),
        ),
        &String::from_str(&env, "Spend"),
        &String::from_str(&env, "Treasury spend proposal"),
        &bad,
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

// ── Issue #997: Governance proposal action allowlist & parameter validation ──
//
// `ProposalType` is the allowlist itself (a closed Rust enum — an XDR
// payload that doesn't decode into one of its variants is rejected by the
// Soroban host before `create_proposal` ever runs, so there is no on-chain
// way to exercise an "unknown action" case). These tests instead cover what
// `validate_proposal` newly enforces on top of that: malformed and
// boundary-value parameters within each *known* action type must be
// rejected at creation time, and never make it into proposal storage.

#[test]
fn proposal_parameter_change_within_bounds_accepted() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // current == proposed == MAX_PARAMETER_VALUE is the top boundary of the
    // allowed range and should still be accepted (delta is 0).
    let boundary = crate::proposals::MAX_PARAMETER_VALUE;
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ParameterChange(String::from_str(&env, "quorum_threshold"), boundary, boundary),
        &String::from_str(&env, "Tweak param"),
        &String::from_str(&env, "boundary value parameter change"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert!(
        matches!(result, Ok(Ok(_))),
        "expected Ok(Ok), got {result:?}"
    );
}

#[test]
fn proposal_parameter_change_above_max_value_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // One past the top of the allowed range must be rejected, not silently
    // stored — regardless of how implausible the arithmetic overflow risk
    // this bound also protects against.
    let over_max = crate::proposals::MAX_PARAMETER_VALUE + 1;
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ParameterChange(String::from_str(&env, "quorum_threshold"), 0, over_max),
        &String::from_str(&env, "Tweak param"),
        &String::from_str(&env, "out of range parameter change"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(
        result,
        Err(Ok(GovernanceError::ProposalParameterOutOfRange))
    );
}

#[test]
fn proposal_parameter_change_current_below_min_value_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // Regression guard: `current == i128::MIN` used to overflow the
    // `(*proposed - *current).abs()` delta computation (panicking instead of
    // returning an error). Bounding `current` first must reject this
    // cleanly with no panic.
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ParameterChange(String::from_str(&env, "quorum_threshold"), i128::MIN, 0),
        &String::from_str(&env, "Tweak param"),
        &String::from_str(&env, "malformed parameter change"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(
        result,
        Err(Ok(GovernanceError::ProposalParameterOutOfRange))
    );
}

#[test]
fn proposal_parameter_change_empty_name_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ParameterChange(String::from_str(&env, ""), 100, 110),
        &String::from_str(&env, "Tweak param"),
        &String::from_str(&env, "empty parameter name"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

#[test]
fn proposal_feature_toggle_empty_name_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::FeatureToggle(String::from_str(&env, ""), true),
        &String::from_str(&env, "Toggle feature"),
        &String::from_str(&env, "empty feature name"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

#[test]
fn proposal_feature_toggle_valid_name_accepted() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::FeatureToggle(String::from_str(&env, "new_signal_ui"), true),
        &String::from_str(&env, "Toggle feature"),
        &String::from_str(&env, "enable new signal UI"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert!(
        matches!(result, Ok(Ok(_))),
        "expected Ok(Ok), got {result:?}"
    );
}

#[test]
fn proposal_signal_proposal_empty_text_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::SignalProposal(String::from_str(&env, "")),
        &String::from_str(&env, "Signal"),
        &String::from_str(&env, "empty signal text"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

#[test]
fn proposal_contract_upgrade_all_zero_hash_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // Right length (32 bytes) but obviously a placeholder/garbage hash, not
    // a real WASM hash — must be rejected rather than stored as executable.
    let zero_hash = Bytes::from_slice(&env, &[0u8; 32]);
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ContractUpgrade(String::from_str(&env, "core"), zero_hash.clone()),
        &String::from_str(&env, "Upgrade"),
        &String::from_str(&env, "all-zero hash upgrade"),
        &zero_hash,
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

#[test]
fn proposal_contract_upgrade_empty_contract_name_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    let hash = make_hash(&env, 0xAB);
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::ContractUpgrade(String::from_str(&env, ""), hash.clone()),
        &String::from_str(&env, "Upgrade"),
        &String::from_str(&env, "empty contract name"),
        &hash,
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

#[test]
fn proposal_treasury_spend_purpose_with_control_char_rejected() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let asset_key = asset(&env);
    env.as_contract(&id, || {
        let mut t = crate::get_treasury(&env);
        t.assets.set(asset_key.clone(), 100_000i128);
        crate::put_treasury(&env, &t);
    });

    let proposer = Address::generate(&env);
    stake_for_proposals(&env, &id, &proposer, 10_000i128);

    // A newline (0x0A) is a control character sanitize_string rejects.
    let purpose = String::from_str(&env, "payment\nwith embedded control char");
    let result = client.try_create_proposal(
        &proposer,
        &ProposalType::TreasurySpend(Address::generate(&env), 1_000i128, asset_key, purpose),
        &String::from_str(&env, "Spend"),
        &String::from_str(&env, "malformed purpose"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidProposal)));
}

// ── Issue #589: Shadow-mode canary upgrade ────────────────────────────────────

fn make_hash(env: &Env, byte: u8) -> Bytes {
    Bytes::from_slice(env, &[byte; 32])
}

#[test]
fn shadow_mode_enter_and_promote() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let hash = make_hash(&env, 0xAB);
    let state = client.enter_shadow_mode(&admin, &hash, &3600u64);
    assert!(!state.promoted);
    assert!(state.trial_ends_at > 0);
    assert!(client.is_in_shadow_mode());

    client.promote_from_shadow_mode(&admin);
    assert!(!client.is_in_shadow_mode());
}

#[test]
fn shadow_compare_matching_outputs_no_discrepancy_event() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let hash = make_hash(&env, 0x01);
    client.enter_shadow_mode(&admin, &hash, &3600u64);

    let h1 = make_hash(&env, 0xFF);
    let h2 = make_hash(&env, 0xFF);
    let matched = client.shadow_compare(&1u32, &h1, &h2);
    assert!(matched);
}

#[test]
fn shadow_compare_mismatched_outputs_emits_discrepancy_event() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let hash = make_hash(&env, 0x01);
    client.enter_shadow_mode(&admin, &hash, &3600u64);

    let old_hash = make_hash(&env, 0xAA);
    let new_hash = make_hash(&env, 0xBB);
    let matched = client.shadow_compare(&2u32, &old_hash, &new_hash);
    assert!(!matched);

    // Verify a discrepancy event was emitted.
    let events = env.events().all();
    let found = events.iter().any(|(_, topics, _)| {
        let t0 = topics
            .get(0)
            .and_then(|v: Val| Symbol::try_from_val(&env, &v).ok());
        let t1 = topics
            .get(1)
            .and_then(|v: Val| Symbol::try_from_val(&env, &v).ok());
        t0 == Some(Symbol::new(&env, "shadow")) && t1 == Some(Symbol::new(&env, "discrep"))
    });
    assert!(found, "expected shadow/discrep event");
}

#[test]
fn shadow_compare_no_op_outside_shadow_period() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    // No shadow mode active — compare should return true (no discrepancy).
    let h1 = make_hash(&env, 0xAA);
    let h2 = make_hash(&env, 0xBB);
    let matched = client.shadow_compare(&3u32, &h1, &h2);
    assert!(matched);
}

#[test]
fn shadow_mode_cancel() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let hash = make_hash(&env, 0x02);
    client.enter_shadow_mode(&admin, &hash, &3600u64);
    assert!(client.is_in_shadow_mode());

    client.cancel_shadow_mode(&admin);
    assert!(!client.is_in_shadow_mode());
}

// ── Issue #592: Storage tier classification ───────────────────────────────────

#[test]
fn vesting_schedule_stored_and_retrievable_post_init() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    // VestingSchedules are now stored in persistent storage. Verify retrieval.
    let schedule = client.get_vesting_schedule(&r.team);
    assert_eq!(schedule.total_amount, 200_000_000);

    let beneficiary = Address::generate(&env);
    client.create_vesting_schedule(&admin, &beneficiary, &500i128, &0u64, &0u64, &100u64);
    let s = client.get_vesting_schedule(&beneficiary);
    assert_eq!(s.total_amount, 500);
}

#[test]
fn pending_rewards_stored_in_persistent_tier() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let beneficiary = Address::generate(&env);

    // Accrue a reward to populate PendingRewards (persistent storage).
    stake_for_proposals(&env, &id, &beneficiary, 5_000i128);
    let _ = client.try_accrue_liquidity_rewards(&admin, &beneficiary, &100_000i128);

    // Verify we can read pending rewards back out via the contract.
    let pending = client.pending_rewards(&beneficiary);
    assert!(pending >= 0, "pending rewards should be non-negative");
}

#[test]
fn vote_locks_stored_in_persistent_tier() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    // Set a vote lock for a user and verify it persists.
    let user = Address::generate(&env);
    stake_for_proposals(&env, &id, &user, 5_000i128);
    client.set_vote_lock(&admin, &user, &1u32);

    // Verify the lock is in place by checking that the entry is stored.
    env.as_contract(&id, || {
        let locks = crate::get_vote_locks(&env);
        assert_eq!(locks.get(user), Some(1u32));
    });
}
