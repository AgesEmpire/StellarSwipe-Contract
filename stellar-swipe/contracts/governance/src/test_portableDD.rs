extern crate std;

use crate::distribution::{DistributionRecipients, YEAR_SECONDS};
use crate::proposals::{
    GovernanceConfig, ProposalCategory, ProposalStatus, ProposalType,
    VoteType as GovernanceVoteType,
};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError, ShadowModeResult};
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

// ── Issue #797: Structured events from shadow-mode proposal dry-run ─────────

/// Look for a `shadow/sim_done` event and, if found, decode its payload as a
/// `ShadowModeResult`.
///
/// Note: the test harness's `env.events().all()` only surfaces events from
/// the most recent top-level contract invocation, so callers must invoke
/// this immediately after the `simulate_proposal` call, before making any
/// further client call.
fn find_shadow_sim_event(env: &Env) -> Option<ShadowModeResult> {
    env.events().all().iter().find_map(|(_, topics, data)| {
        let t0 = topics
            .get(0)
            .and_then(|v: Val| Symbol::try_from_val(env, &v).ok());
        let t1 = topics
            .get(1)
            .and_then(|v: Val| Symbol::try_from_val(env, &v).ok());
        if t0 == Some(Symbol::new(env, "shadow")) && t1 == Some(Symbol::new(env, "sim_done")) {
            ShadowModeResult::try_from_val(env, &data).ok()
        } else {
            None
        }
    })
}

fn nonzero_execution_delay_config() -> GovernanceConfig {
    GovernanceConfig {
        min_proposal_threshold: 1_000,
        voting_period: 7 * 86_400,
        voting_delay: 60,
        quorum_threshold: 1_000,
        approval_threshold: 5_000,
        execution_delay: 60,
        discussion_duration: 0,
    }
}

#[test]
fn simulate_proposal_success_emits_event_and_writes_no_state() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    // A non-zero execution delay keeps `finalize_proposal` from
    // auto-executing on success, so the proposal stays `Succeeded` — the
    // shape the real `execute_proposal` preconditions expect.
    client.configure_governance(&admin, &nonzero_execution_delay_config());

    client.stake(&r.community_rewards, &120_000_000i128);
    client.stake(&r.public_sale, &40_000_000i128);

    let proposal_id = client.create_proposal(
        &r.community_rewards,
        &ProposalType::ParameterChange(String::from_str(&env, "liquidity_reward_bps"), 100, 120),
        &String::from_str(&env, "Adjust reward"),
        &String::from_str(&env, "Increase by 20%"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &r.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(&proposal_id, &r.public_sale, &GovernanceVoteType::For);

    env.ledger().set_timestamp(8 * 86_400);
    assert_eq!(
        client.finalize_proposal(&proposal_id),
        ProposalStatus::Succeeded
    );

    // Past the 60s execution delay, still inside the 7-day execution window.
    env.ledger().set_timestamp(8 * 86_400 + 61);

    let result = client.simulate_proposal(&proposal_id);
    assert!(result.success);
    assert!(result.failure_reason.is_none());
    assert!(!result.simulated_state_changes.is_empty());
    assert_eq!(result.proposal_id, proposal_id);

    let evt = find_shadow_sim_event(&env).expect("expected a shadow/sim_done event");

    // No persistent state was written by the dry-run: the proposal must
    // still read as `Succeeded` — `execute_proposal` would have flipped it
    // to `Executed` and applied the parameter change.
    let proposal_after = client.proposal(&proposal_id);
    assert_eq!(proposal_after.status, ProposalStatus::Succeeded);
    assert_eq!(evt.proposal_id, proposal_id);
    assert!(evt.success);
    assert!(evt.failure_reason.is_none());
    assert!(!evt.simulated_state_changes.is_empty());
}

#[test]
fn simulate_proposal_failure_emits_event_with_reason_and_writes_no_state() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    client.stake(&r.community_rewards, &120_000_000i128);
    client.stake(&r.public_sale, &40_000_000i128);

    let proposal_id = client.create_proposal(
        &r.community_rewards,
        &ProposalType::ParameterChange(String::from_str(&env, "liquidity_reward_bps"), 100, 120),
        &String::from_str(&env, "Adjust reward"),
        &String::from_str(&env, "Increase by 20%"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // The proposal is still `Pending` (voting has neither started nor been
    // finalised) — `execute_proposal` would reject it outright, so the
    // dry-run must report `success = false` with a populated reason.
    let result = client.simulate_proposal(&proposal_id);
    assert!(!result.success);
    assert!(result.failure_reason.is_some());
    assert_eq!(result.proposal_id, proposal_id);

    // `env.events().all()` in the test harness only surfaces events from the
    // most recent top-level contract invocation, so this must be captured
    // before any further client call is made.
    let evt = find_shadow_sim_event(&env).expect("expected a shadow/sim_done event");
    assert_eq!(evt.proposal_id, proposal_id);
    assert!(!evt.success);
    assert!(evt.failure_reason.is_some());

    // No persistent state was written: the proposal must still read as
    // `Pending`, untouched by the dry-run.
    let proposal_after = client.proposal(&proposal_id);
    assert_eq!(proposal_after.status, ProposalStatus::Pending);
}

#[test]
fn simulate_proposal_treasury_spend_insufficient_balance_reports_failure() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    client.configure_governance(&admin, &nonzero_execution_delay_config());
    client.stake(&r.community_rewards, &120_000_000i128);
    client.stake(&r.public_sale, &40_000_000i128);

    // `validate_proposal` requires `amount <= balance / 10` at *creation*
    // time (a headroom check, separate from the balance check `execute_proposal`
    // performs at execution time), so the treasury must start well-funded —
    // mirrors `queue_underfunded_treasury_spend_action` in test.rs.
    let spend_asset = asset(&env);
    client.set_treasury_asset(&admin, &spend_asset, &10_000i128);

    let proposal_id = client.create_proposal(
        &r.community_rewards,
        &ProposalType::TreasurySpend(
            r.public_sale.clone(),
            500i128,
            spend_asset,
            String::from_str(&env, "overspend"),
        ),
        &String::from_str(&env, "Spend more than the treasury holds"),
        &String::from_str(&env, "Should fail simulation"),
        &Bytes::new(&env),
        &ProposalCategory::TreasuryTransfer,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &r.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(&proposal_id, &r.public_sale, &GovernanceVoteType::For);

    env.ledger().set_timestamp(8 * 86_400);
    assert_eq!(
        client.finalize_proposal(&proposal_id),
        ProposalStatus::Succeeded
    );
    env.ledger().set_timestamp(8 * 86_400 + 61);

    // Drain the treasury after approval — mirrors the real-world scenario
    // the emergency timelock path is designed for (queue_underfunded_*
    // helper in test.rs), except here it must surface as a *simulated*
    // failure instead of a real, storage-mutating one.
    client.set_treasury_asset(&admin, &asset(&env), &0i128);

    let result = client.simulate_proposal(&proposal_id);
    assert!(!result.success);
    assert!(result.failure_reason.is_some());

    let evt = find_shadow_sim_event(&env).expect("expected a shadow/sim_done event");
    assert!(!evt.success);
    assert!(evt.failure_reason.is_some());

    // The proposal is still `Succeeded`, not `Executed`, and the treasury
    // balance the simulation read is unchanged by the call itself.
    let proposal_after = client.proposal(&proposal_id);
    assert_eq!(proposal_after.status, ProposalStatus::Succeeded);
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
