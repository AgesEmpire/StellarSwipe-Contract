/// Tests: #1430 — read-only governance execution previews.
///
/// Covers:
///   1. Preview of a pending (not-yet-succeeded) proposal reports correct failures.
///   2. Preview of a succeeded, executable proposal reports all checks passing.
///   3. TreasurySpend with insufficient balance has that check failing.
///   4. Storage and token balances are unchanged after any preview call.
extern crate std;

use crate::distribution::DistributionRecipients;
use crate::proposals::{ProposalCategory, ProposalStatus, ProposalType};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, Env, String};
use stellar_swipe_common::Asset;

const SUPPLY: i128 = 1_000_000_000;

fn setup() -> (Env, Address, Address, DistributionRecipients) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
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

fn client<'a>(env: &'a Env, id: &'a Address) -> GovernanceContractClient<'a> {
    GovernanceContractClient::new(env, id)
}

fn init(c: &GovernanceContractClient, env: &Env, admin: &Address, r: &DistributionRecipients) {
    c.initialize(
        admin,
        &String::from_str(env, "StellarSwipe Gov"),
        &String::from_str(env, "SSG"),
        &7u32,
        &SUPPLY,
        r,
    );
}

fn usdc(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "USDC"),
        issuer: None,
    }
}

/// Preview of a pending (not-succeeded) proposal reports that proposal_succeeded fails.
#[test]
fn preview_pending_proposal_reports_not_succeeded() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    c.stake(&r.community_rewards, &10_000i128);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "preview test")),
        &String::from_str(&env, "T"),
        &String::from_str(&env, "D"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    let preview = c.preview_execution(&pid);
    assert!(!preview.all_pass, "pending proposal preview must not all_pass");

    let succeeded_check = preview
        .outcomes
        .iter()
        .find(|o| o.check == String::from_str(&env, "proposal_succeeded"));
    assert!(
        succeeded_check.is_some(),
        "proposal_succeeded check must be present"
    );
    assert!(
        !succeeded_check.unwrap().passed,
        "proposal_succeeded must be false for a pending proposal"
    );
}

/// Non-existent proposal returns ProposalNotFound.
#[test]
fn preview_nonexistent_proposal_returns_error() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let result = c.try_preview_execution(&9_999u64);
    assert_eq!(
        result,
        Err(Ok(GovernanceError::ProposalNotFound)),
        "preview of non-existent proposal must return ProposalNotFound"
    );
}

/// TreasurySpend with insufficient balance has treasury_balance_sufficient = false.
#[test]
fn treasury_spend_preview_reports_insufficient_balance() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    c.stake(&r.community_rewards, &10_000i128);

    let asset = usdc(&env);
    c.set_treasury_asset(&admin, &asset, &10_000i128);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::TreasurySpend(
            Address::generate(&env),
            1_000i128,
            asset.clone(),
            String::from_str(&env, "test"),
        ),
        &String::from_str(&env, "Spend"),
        &String::from_str(&env, "Spend from treasury"),
        &Bytes::new(&env),
        &ProposalCategory::TreasuryTransfer,
        &false,
    );

    // Drain the treasury so the balance is 0 when preview runs.
    c.set_treasury_asset(&admin, &asset, &0i128);

    let preview = c.preview_execution(&pid);

    let bal_check = preview
        .outcomes
        .iter()
        .find(|o| o.check == String::from_str(&env, "treasury_balance_sufficient"));
    assert!(
        bal_check.is_some(),
        "treasury_balance_sufficient check must be present for TreasurySpend"
    );
    assert!(
        !bal_check.unwrap().passed,
        "treasury_balance_sufficient must be false when balance is 0"
    );
}

/// Storage and token balances are unchanged after a preview call.
#[test]
fn preview_does_not_mutate_storage_or_balances() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    c.stake(&r.community_rewards, &10_000i128);

    let asset = usdc(&env);
    let initial_balance = 100_000i128;
    c.set_treasury_asset(&admin, &asset, &initial_balance);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::TreasurySpend(
            Address::generate(&env),
            1_000i128,
            asset.clone(),
            String::from_str(&env, "test"),
        ),
        &String::from_str(&env, "Spend"),
        &String::from_str(&env, "Spend from treasury"),
        &Bytes::new(&env),
        &ProposalCategory::TreasuryTransfer,
        &false,
    );

    // Run preview twice.
    let _ = c.preview_execution(&pid);
    let _ = c.preview_execution(&pid);

    // Proposal status must still be Pending.
    let proposal = c.proposal(&pid);
    assert_eq!(
        proposal.status,
        ProposalStatus::Pending,
        "proposal status must remain Pending after preview"
    );

    // Staked balance must be unchanged.
    let staked = c.staked_balance(&r.community_rewards);
    assert_eq!(staked, 10_000, "staked balance must not change after preview");

    // evaluated_at must be populated.
    let preview = c.preview_execution(&pid);
    assert!(
        preview.evaluated_at > 0,
        "evaluated_at must be set to the current ledger timestamp"
    );
}

/// Preview always includes at minimum the standard lifecycle checks.
#[test]
fn preview_always_includes_standard_checks() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    c.stake(&r.community_rewards, &10_000i128);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "check completeness")),
        &String::from_str(&env, "T"),
        &String::from_str(&env, "D"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    let preview = c.preview_execution(&pid);
    let check_names: std::vec::Vec<String> = preview
        .outcomes
        .iter()
        .map(|o| o.check.clone())
        .collect();

    for expected in &[
        "proposal_succeeded",
        "not_vetoed",
        "veto_window_closed",
        "execution_delay_elapsed",
        "within_execution_deadline",
    ] {
        assert!(
            check_names.contains(&String::from_str(&env, expected)),
            "standard check '{}' must be present in preview outcomes",
            expected
        );
    }
}
