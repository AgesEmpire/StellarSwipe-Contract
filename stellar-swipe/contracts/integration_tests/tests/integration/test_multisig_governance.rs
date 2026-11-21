#![cfg(test)]

use signal_registry::{CriticalActionPayload, SignalRegistry, SignalRegistryClient};
use soroban_sdk::{testutils::Address as _, testutils::Ledger, vec, Address, Env, String};
use stellar_swipe_common::{MultisigTimelockConfig, ProposalStatus};

#[test]
fn test_multisig_governance_full_flow() {
    let env = Env::default();
    env.mock_all_auths();

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);

    client.initialize(&admin);

    let signers = vec![&env, alice.clone(), bob.clone(), carol.clone()];
    client.enable_multisig(&admin, &signers, &2);

    let mut config = MultisigTimelockConfig::default_config();
    config.parameter_delay = 0;
    config.unpause_delay = 0;
    client.set_multisig_timelock_config(&alice, &config);

    // 1. Propose parameter update (min stake)
    let proposal_id =
        client.propose_critical_action(&alice, &CriticalActionPayload::SetMinStake(500_000_000));
    assert_eq!(
        client.get_approval_proposal(&proposal_id).status,
        ProposalStatus::Pending
    );

    // 2. Second signer approves -> threshold met
    assert_eq!(
        client.approve_proposal(&bob, &proposal_id),
        ProposalStatus::Approved
    );

    // 3. Execute after timelock (zero delay configured)
    client.execute_proposal(&carol, &proposal_id);
    assert_eq!(client.get_config().min_stake, 500_000_000);

    // 4. Pause via multisig, then unpause via separate proposal
    config.pause_delay = 0;
    client.set_multisig_timelock_config(&alice, &config);

    let pause_id = client.propose_critical_action(
        &alice,
        &CriticalActionPayload::PauseCategory(
            String::from_str(&env, "trading"),
            None,
            String::from_str(&env, "Integration test pause"),
        ),
    );
    client.approve_proposal(&carol, &pause_id);
    client.execute_proposal(&bob, &pause_id);
    assert!(client.is_paused());

    let unpause_id = client.propose_critical_action(
        &bob,
        &CriticalActionPayload::UnpauseCategory(String::from_str(&env, "trading")),
    );
    client.approve_proposal(&alice, &unpause_id);
    client.execute_proposal(&carol, &unpause_id);
    assert!(!client.is_paused());
}
