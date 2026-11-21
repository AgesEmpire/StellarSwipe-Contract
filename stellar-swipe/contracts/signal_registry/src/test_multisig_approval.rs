#![cfg(test)]

use super::*;
use crate::errors::AdminError;
use crate::CriticalActionPayload;
use soroban_sdk::{testutils::Address as _, testutils::Ledger, vec, Env, String};
use stellar_swipe_common::{MultisigTimelockConfig, ProposalStatus, DEFAULT_FEE_CHANGE_DELAY};

fn setup_multisig_client(
    env: &Env,
) -> (SignalRegistryClient<'_>, Address, Address, Address, Address) {
    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(env, &contract_id);

    let admin = Address::generate(env);
    let signer1 = Address::generate(env);
    let signer2 = Address::generate(env);
    let signer3 = Address::generate(env);

    client.initialize(&admin);

    let signers = vec![env, signer1.clone(), signer2.clone(), signer3.clone()];
    client.enable_multisig(&admin, &signers, &2);

    (client, signer1, signer2, signer3, admin)
}

#[test]
fn test_direct_critical_ops_blocked_when_multisig_enabled() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, _, _, _) = setup_multisig_client(&env);

    let result = client.try_set_trade_fee(&signer1, &25);
    assert_eq!(result, Err(Ok(AdminError::RequiresMultisigApproval)));
}

#[test]
fn test_fee_change_2_of_3_with_timelock() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, signer2, _, _) = setup_multisig_client(&env);

    let mut config = MultisigTimelockConfig::default_config();
    config.fee_change_delay = 0;
    client.set_multisig_timelock_config(&signer1, &config);

    let payload = CriticalActionPayload::SetTradeFee(25);
    let proposal_id = client.propose_critical_action(&signer1, &payload);

    let proposal = client.get_approval_proposal(&proposal_id);
    assert_eq!(proposal.approvals.len(), 1);
    assert_eq!(proposal.status, ProposalStatus::Pending);

    let status = client.approve_proposal(&signer2, &proposal_id);
    assert_eq!(status, ProposalStatus::Approved);

    client.execute_proposal(&signer2, &proposal_id);
    assert_eq!(client.get_config().trade_fee_bps, 25);
}

#[test]
fn test_insufficient_approvals_cannot_execute() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, _, _, _) = setup_multisig_client(&env);

    let payload = CriticalActionPayload::SetTradeFee(30);
    let proposal_id = client.propose_critical_action(&signer1, &payload);

    let result = client.try_execute_proposal(&signer1, &proposal_id);
    assert_eq!(result, Err(Ok(AdminError::ProposalNotApproved)));
}

#[test]
fn test_timelock_blocks_early_execution() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, signer2, _, _) = setup_multisig_client(&env);

    let payload = CriticalActionPayload::SetTradeFee(15);
    let proposal_id = client.propose_critical_action(&signer1, &payload);
    client.approve_proposal(&signer2, &proposal_id);

    let result = client.try_execute_proposal(&signer1, &proposal_id);
    assert_eq!(result, Err(Ok(AdminError::TimelockNotElapsed)));

    let now = env.ledger().timestamp();
    env.ledger()
        .set_timestamp(now + DEFAULT_FEE_CHANGE_DELAY + 1);

    client.execute_proposal(&signer1, &proposal_id);
    assert_eq!(client.get_config().trade_fee_bps, 15);
}

#[test]
fn test_cancel_proposal() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, signer2, _, _) = setup_multisig_client(&env);

    let payload = CriticalActionPayload::SetTradeFee(20);
    let proposal_id = client.propose_critical_action(&signer1, &payload);
    client.cancel_proposal(&signer2, &proposal_id);

    let result = client.try_approve_proposal(&signer2, &proposal_id);
    assert_eq!(result, Err(Ok(AdminError::ProposalCancelled)));
}

#[test]
fn test_duplicate_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, _, _, _) = setup_multisig_client(&env);

    let payload = CriticalActionPayload::SetMinStake(200_000_000);
    let proposal_id = client.propose_critical_action(&signer1, &payload);

    let result = client.try_approve_proposal(&signer1, &proposal_id);
    assert_eq!(result, Err(Ok(AdminError::AlreadyApproved)));
}

#[test]
fn test_guardian_emergency_pause_bypasses_multisig() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, signer2, _, _) = setup_multisig_client(&env);
    let guardian = Address::generate(&env);

    let mut config = MultisigTimelockConfig::default_config();
    config.guardian_delay = 0;
    client.set_multisig_timelock_config(&signer1, &config);

    let proposal_id = client.propose_critical_action(
        &signer1,
        &CriticalActionPayload::SetGuardian(guardian.clone()),
    );
    client.approve_proposal(&signer2, &proposal_id);
    client.execute_proposal(&signer2, &proposal_id);

    client.pause_trading(&guardian);
    assert!(client.is_paused());
}

#[test]
fn test_pause_via_multisig_proposal() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, signer1, signer2, _, _) = setup_multisig_client(&env);

    let mut config = MultisigTimelockConfig::default_config();
    config.pause_delay = 0;
    client.set_multisig_timelock_config(&signer1, &config);

    let category = String::from_str(&env, "trading");
    let reason = String::from_str(&env, "Maintenance");
    let payload = CriticalActionPayload::PauseCategory(category, None, reason);

    let proposal_id = client.propose_critical_action(&signer1, &payload);
    client.approve_proposal(&signer2, &proposal_id);
    client.execute_proposal(&signer2, &proposal_id);

    assert!(client.is_paused());
}
