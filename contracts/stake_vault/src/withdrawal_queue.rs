use soroban_sdk::{contracttype, token, Address, Env, Vec};

use crate::storage::{get_admin, get_token, DataKey};

/// A single queued withdrawal request.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawalRequest {
    pub owner: Address,
    pub shares: i128,
    pub assets: i128,
    pub unlock_ledger: u32,
}

/// Append a withdrawal request to the queue for `owner`.
pub fn enqueue(env: &Env, owner: Address, shares: i128, assets: i128, unlock_ledger: u32) {
    let mut queue: Vec<WithdrawalRequest> = env
        .storage()
        .persistent()
        .get(&DataKey::WithdrawalQueue)
        .unwrap_or_else(|| Vec::new(env));

    queue.push_back(WithdrawalRequest {
        owner,
        shares,
        assets,
        unlock_ledger,
    });

    env.storage()
        .persistent()
        .set(&DataKey::WithdrawalQueue, &queue);
}

/// Return the full withdrawal queue.
pub fn get_queue(env: &Env) -> Vec<WithdrawalRequest> {
    env.storage()
        .persistent()
        .get(&DataKey::WithdrawalQueue)
        .unwrap_or_else(|| Vec::new(env))
}

/// Process all matured withdrawal requests, transferring assets to owners.
///
/// Only the admin may invoke processing. Each matured request is settled via
/// the Soroban token interface (`token::Client::transfer`), which enforces the
/// vault's authorization for the outgoing transfer.
pub fn process_queue(env: &Env) -> Vec<WithdrawalRequest> {
    let admin = get_admin(env);
    admin.require_auth();

    let token_address = get_token(env);
    let token_client = token::Client::new(env, &token_address);

    let current_ledger = env.ledger().sequence();
    let queue = get_queue(env);
    let mut remaining: Vec<WithdrawalRequest> = Vec::new(env);
    let mut processed: Vec<WithdrawalRequest> = Vec::new(env);

    for request in queue.iter() {
        if request.unlock_ledger <= current_ledger {
            token_client.transfer(
                &env.current_contract_address(),
                &request.owner,
                &request.assets,
            );
            processed.push_back(request.clone());
        } else {
            remaining.push_back(request);
        }
    }

    env.storage()
        .persistent()
        .set(&DataKey::WithdrawalQueue, &remaining);

    processed
}

/// Deposit assets into the vault on behalf of `from` using `transfer_from`.
///
/// The caller must authorize the transfer; the token client enforces the
/// allowance and signature checks for the incoming transfer.
pub fn deposit_assets(env: &Env, from: Address, amount: i128) {
    from.require_auth();

    let token_address = get_token(env);
    let token_client = token::Client::new(env, &token_address);

    token_client.transfer_from(
        &env.current_contract_address(),
        &from,
        &env.current_contract_address(),
        &amount,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    pub struct MockToken;

    #[contractimpl]
    impl MockToken {
        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            env.storage()
                .persistent()
                .set(&(from.clone(), to.clone()), &amount);
        }

        pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
            spender.require_auth();
            from.require_auth();
            env.storage()
                .persistent()
                .set(&(from.clone(), to.clone()), &amount);
        }
    }

    fn setup(env: &Env) -> (Address, Address) {
        let admin = Address::generate(env);
        let token_id = env.register_contract(None, MockToken);
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Token, &token_id);
        (admin, token_id)
    }

    #[test]
    fn test_process_queue_transfers_matured() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, token_id) = setup(&env);
        let owner = Address::generate(&env);

        env.ledger().set_sequence_number(100);
        enqueue(&env, owner.clone(), 10, 50, 90);

        let processed = process_queue(&env);
        assert_eq!(processed.len(), 1);
        assert_eq!(get_queue(&env).len(), 0);

        let client = token::Client::new(&env, &token_id);
        let _ = client;
        let _ = admin;
    }

    #[test]
    fn test_process_queue_keeps_unmatured() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, _token_id) = setup(&env);
        let owner = Address::generate(&env);

        env.ledger().set_sequence_number(10);
        enqueue(&env, owner, 10, 50, 90);

        let processed = process_queue(&env);
        assert_eq!(processed.len(), 0);
        assert_eq!(get_queue(&env).len(), 1);
    }

    #[test]
    fn test_deposit_assets_transfer_from() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, _token_id) = setup(&env);
        let from = Address::generate(&env);

        deposit_assets(&env, from, 100);
    }
}
