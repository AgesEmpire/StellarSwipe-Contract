//! Cross-chain transaction monitoring and finality verification.
//!
//! Monitors source chain deposits for transaction finality before minting,
//! handling different finality rules per chain and reorganizations.

#![allow(dead_code)]

use soroban_sdk::{contracttype, String, Symbol, Vec, Env, Address};
use stellar_swipe_common::assets::Asset;
use crate::analytics::{update_transfer_analytics, update_validator_analytics};

/// Chain identifiers for multi-chain support
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainId {
    Stellar,
    Ethereum,
    Bitcoin,
    Polygon,
    BNB,
}

/// Different finality achievement methods across chains
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationMethod {
    BlockConfirmations,  // Simple: wait N blocks
    EpochFinality,       // PoS: wait for epoch finalization
    Probabilistic,       // Bitcoin: 6+ confirmations
}

/// Finality configuration per chain
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainFinalityConfig {
    pub chain_id: ChainId,
    pub required_confirmations: u32,
    pub average_block_time: u64,      // seconds
    pub reorg_depth_limit: u32,       // Max reorg depth
    pub verification_method: VerificationMethod,
}

/// Transaction monitoring statuses
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitoringStatus {
    Pending,      // Initial state, waiting to see on chain
    Confirming,   // Seen on chain, awaiting confirmations
    Finalized,    // Sufficient confirmations received
    Reorged,      // Transaction was reorganized
    Failed,       // Monitoring timeout or error
}

/// Monitored cross-chain transaction
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitoredTransaction {
    pub transfer_id: u64,
    pub source_chain: ChainId,
    pub tx_hash: String,
    pub block_number: u64,
    pub confirmations: u32,
    pub status: MonitoringStatus,
    pub first_seen: u64,
    pub finalized_at: Option<u64>,
}

/// Bridge transfer tracking
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeTransfer {
    pub transfer_id: u64,
    pub bridge_id: u64,
    pub source_chain: ChainId,
    pub destination_chain: ChainId,
    pub amount: i128,
    pub fee_paid: i128,
    pub stellar_asset: Asset,
    pub user: String,
    pub status: TransferStatus,
    pub validator_signatures: Vec<String>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
}

/// Transfer status tracking
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferStatus {
    Pending,          // Awaiting source finality
    Finalized,        // Source finalized, awaiting validators
    ValidatorApproved, // Validators approved
    Minting,          // In progress
    Complete,         // Successfully minted
    Failed,           // Transfer failed
}

/// Storage keys for persistence
#[contracttype]
pub enum MonitoringDataKey {
    MonitoredTx(u64),                      // By transfer_id
    BridgeTransfer(u64),                   // By transfer_id
    ChainConfig(u32),                      // By chain discriminant
    PendingTransactions,                   // List of pending transfer IDs
    TransactionIndex(u64),                 // Meta index
}

// Constants for finality configurations
const ETHEREUM_FINALITY: u32 = 32;  // 32 blocks (~6.4 min)
const POLYGON_FINALITY: u32 = 128;  // 128 blocks (~4.3 min)
const BSC_FINALITY: u32 = 15;       // 15 blocks (~45 sec)
const BITCOIN_FINALITY: u32 = 6;    // 6 blocks (~60 min)

const ETHEREUM_BLOCK_TIME: u64 = 12;  // seconds
const POLYGON_BLOCK_TIME: u64 = 2;    // seconds
const BSC_BLOCK_TIME: u64 = 3;        // seconds
const BITCOIN_BLOCK_TIME: u64 = 600;  // seconds

const MONITORING_TIMEOUT: u64 = 3600;  // 1 hour in seconds

/// ==========================
/// Chain Configuration
/// ==========================

/// Get finality configuration for a chain
pub fn get_chain_finality_config(env: &Env, chain_id: ChainId) -> Result<ChainFinalityConfig, String> {
    let chain_key = match chain_id {
        ChainId::Ethereum => 1u32,
        ChainId::Bitcoin => 2,
        ChainId::Polygon => 3,
        ChainId::BNB => 4,
        ChainId::Stellar => return Err(String::from_linear(env, "Stellar has no finality config")),
    };

    if let Some(config) = env
        .storage()
        .persistent()
        .get::<MonitoringDataKey, ChainFinalityConfig>(&MonitoringDataKey::ChainConfig(
            chain_key,
        ))
    {
        Ok(config)
    } else {
        // Return default config for chain
        Ok(get_default_config(chain_id))
    }
}

/// Get default finality config for a chain
fn get_default_config(chain_id: ChainId) -> ChainFinalityConfig {
    match chain_id {
        ChainId::Ethereum => ChainFinalityConfig {
            chain_id,
            required_confirmations: ETHEREUM_FINALITY,
            average_block_time: ETHEREUM_BLOCK_TIME,
            reorg_depth_limit: 64,
            verification_method: VerificationMethod::EpochFinality,
        },
        ChainId::Bitcoin => ChainFinalityConfig {
            chain_id,
            required_confirmations: BITCOIN_FINALITY,
            average_block_time: BITCOIN_BLOCK_TIME,
            reorg_depth_limit: 10,
            verification_method: VerificationMethod::Probabilistic,
        },
        ChainId::Polygon => ChainFinalityConfig {
            chain_id,
            required_confirmations: POLYGON_FINALITY,
            average_block_time: POLYGON_BLOCK_TIME,
            reorg_depth_limit: 256,
            verification_method: VerificationMethod::BlockConfirmations,
        },
        ChainId::BNB => ChainFinalityConfig {
            chain_id,
            required_confirmations: BSC_FINALITY,
            average_block_time: BSC_BLOCK_TIME,
            reorg_depth_limit: 20,
            verification_method: VerificationMethod::BlockConfirmations,
        },
        ChainId::Stellar => ChainFinalityConfig {
            chain_id,
            required_confirmations: 1,
            average_block_time: 5,
            reorg_depth_limit: 0,
            verification_method: VerificationMethod::BlockConfirmations,
        },
    }
}

/// Store custom finality config
pub fn set_chain_finality_config(env: &Env, config: &ChainFinalityConfig) {
    let chain_key = match config.chain_id {
        ChainId::Ethereum => 1u32,
        ChainId::Bitcoin => 2,
        ChainId::Polygon => 3,
        ChainId::BNB => 4,
        ChainId::Stellar => return,
    };

    env.storage()
        .persistent()
        .set(&MonitoringDataKey::ChainConfig(chain_key), config);
}

/// ==========================
/// Transaction Monitoring
/// ==========================

/// Start monitoring a source chain transaction
pub fn monitor_source_transaction(
    env: &Env,
    transfer_id: u64,
    tx_hash: String,
    source_chain: ChainId,
    block_number: u64,
) -> Result<(), String> {
    // Verify finality config exists
    let _config = get_chain_finality_config(env, source_chain)?;

    // Create monitored transaction
    let monitored = MonitoredTransaction {
        transfer_id,
        source_chain,
        tx_hash: tx_hash.clone(),
        block_number,
        confirmations: 0,
        status: MonitoringStatus::Pending,
        first_seen: env.ledger().timestamp(),
        finalized_at: None,
    };

    // Store monitored transaction
    store_monitored_tx(env, transfer_id, &monitored);

    // Emit event
    env.events().publish(
        (Symbol::new(env, "transaction_monitoring_started"), transfer_id),
        (source_chain as u32, tx_hash),
    );

    Ok(())
}

/// Get a monitored transaction
pub fn get_monitored_tx(env: &Env, transfer_id: u64) -> Option<MonitoredTransaction> {
    env.storage()
        .persistent()
        .get(&MonitoringDataKey::MonitoredTx(transfer_id))
}

/// Store a monitored transaction
fn store_monitored_tx(env: &Env, transfer_id: u64, monitored: &MonitoredTransaction) {
    env.storage()
        .persistent()
        .set(&MonitoringDataKey::MonitoredTx(transfer_id), monitored);
}

/// Get all pending monitored transactions (limited query)
pub fn get_pending_monitored_transactions(env: &Env, limit: u32) -> Vec<MonitoredTransaction> {
    let mut results = Vec::new(env);

    // In real implementation, would iterate through transaction index
    // For now, return empty - would be populated by oracle/monitoring service
    results
}

/// ==========================
/// Confirmation Tracking
/// ==========================

/// Update confirmations for all pending transactions
///
/// Called periodically by oracle or monitoring service
pub fn update_transaction_confirmations(
    env: &Env,
    current_blocks: Vec<(u32, u64)>, // (chain_id, current_block_number)
) -> Result<Vec<u64>, String> {
    let mut finalized_transfers = Vec::new(env);

    // Process each pending transaction would iterate here
    // This function demonstrates the logic pattern

    Ok(finalized_transfers)
}

/// Update a specific transaction's confirmation count
pub fn update_transaction_confirmation_count(
    env: &Env,
    transfer_id: u64,
    current_block: u64,
) -> Result<bool, String> {
    let mut monitored = get_monitored_tx(env, transfer_id)
        .ok_or_else(|| String::from_linear(env, "Transaction not found"))?;

    let finality_config = get_chain_finality_config(env, monitored.source_chain)?;

    // Calculate confirmations
    let confirmations = current_block.saturating_sub(monitored.block_number) as u32;
    monitored.confirmations = confirmations;

    let is_finalized = match finality_config.verification_method {
        VerificationMethod::BlockConfirmations => {
            confirmations >= finality_config.required_confirmations
        }
        VerificationMethod::EpochFinality => {
            // Assume epoch finality established after certain confirmations
            confirmations >= finality_config.required_confirmations / 2
        }
        VerificationMethod::Probabilistic => {
            // Require double confirmations for probabilistic finality
            confirmations >= finality_config.required_confirmations * 2
        }
    };

    if is_finalized && monitored.status != MonitoringStatus::Finalized {
        mark_as_finalized(env, &mut monitored)?;
        return Ok(true);
    } else if monitored.status != MonitoringStatus::Finalized {
        monitored.status = MonitoringStatus::Confirming;
        store_monitored_tx(env, transfer_id, &monitored);
    }

    Ok(false)
}

/// Mark transaction as finalized
fn mark_as_finalized(env: &Env, monitored: &mut MonitoredTransaction) -> Result<(), String> {
    monitored.status = MonitoringStatus::Finalized;
    monitored.finalized_at = Some(env.ledger().timestamp());

    store_monitored_tx(env, monitored.transfer_id, monitored);

    // Emit finalization event
    env.events().publish(
        (
            Symbol::new(env, "transaction_finalized"),
            monitored.transfer_id,
        ),
        monitored.confirmations,
    );

    Ok(())
}

/// ==========================
/// Reorganization Handling
/// ==========================

/// Check if transaction has been reorganized
pub fn check_for_reorg(
    env: &Env,
    transfer_id: u64,
    current_block: u64,
) -> Result<bool, String> {
    let monitored = get_monitored_tx(env, transfer_id)
        .ok_or_else(|| String::from_linear(env, "Transaction not found"))?;

    let finality_config = get_chain_finality_config(env, monitored.source_chain)?;

    // Reorg detection: check if current block is too far back
    // A reorg is indicated if we're querying a block that's within reorg_depth of current
    if current_block <= monitored.block_number {
        emit_reorg_event(env, transfer_id, monitored.block_number, current_block);
        return Ok(true);
    }

    // Check if within potential reorg depth
    let blocks_since = current_block.saturating_sub(monitored.block_number);
    if blocks_since < finality_config.reorg_depth_limit as u64 {
        // Still in potential reorg zone
        // This would normally query the chain to verify transaction still exists
        return Ok(false);
    }

    // Beyond reorg depth - safe from reorganization
    Ok(false)
}

/// Handle a detected reorganization
pub fn handle_reorg(env: &Env, transfer_id: u64) -> Result<(), String> {
    let mut monitored = get_monitored_tx(env, transfer_id)
        .ok_or_else(|| String::from_linear(env, "Transaction not found"))?;

    // Reset monitoring state
    monitored.confirmations = 0;
    monitored.status = MonitoringStatus::Reorged;
    store_monitored_tx(env, transfer_id, &monitored);

    // Reset transfer status if it exists
    if let Some(mut transfer) = get_bridge_transfer(env, transfer_id) {
        transfer.status = TransferStatus::Pending;
        transfer.validator_signatures = Vec::new(env);
        store_bridge_transfer(env, &transfer);

        env.events().publish(
            (Symbol::new(env, "transfer_reset_reorg"), transfer_id),
            env.ledger().timestamp(),
        );
    }

    // Emit reorg handled event
    env.events().publish(
        (Symbol::new(env, "reorg_handled"), transfer_id),
        monitored.confirmations,
    );

    Ok(())
}

fn emit_reorg_event(env: &Env, transfer_id: u64, old_block: u64, new_block: u64) {
    env.events().publish(
        (Symbol::new(env, "reorg_detected"), transfer_id),
        (old_block, new_block),
    );
}

/// ==========================
/// Timeout Handling
/// ==========================

/// Check for monitoring timeouts on pending transactions
pub fn check_monitoring_timeouts(env: &Env, limit: u32) -> Result<Vec<u64>, String> {
    let mut failed_transfers = Vec::new(env);
    let current_time = env.ledger().timestamp();

    // In real implementation, would iterate through pending transactions
    // This demonstrates the logic

    Ok(failed_transfers)
}

/// Mark transaction as failed due to timeout
pub fn mark_transaction_failed(env: &Env, transfer_id: u64) -> Result<(), String> {
    let mut monitored = get_monitored_tx(env, transfer_id)
        .ok_or_else(|| String::from_linear(env, "Transaction not found"))?;

    monitored.status = MonitoringStatus::Failed;
    store_monitored_tx(env, transfer_id, &monitored);

    // Update transfer status
    if let Some(mut transfer) = get_bridge_transfer(env, transfer_id) {
        transfer.status = TransferStatus::Failed;
        store_bridge_transfer(env, &transfer);
    }

    env.events().publish(
        (Symbol::new(env, "monitoring_failed"), transfer_id),
        current_time(env),
    );

    Ok(())
}

/// ==========================
/// Bridge Transfer Management
/// ==========================

/// Get bridge transfer
pub fn get_bridge_transfer(env: &Env, transfer_id: u64) -> Option<BridgeTransfer> {
    env.storage()
        .persistent()
        .get(&MonitoringDataKey::BridgeTransfer(transfer_id))
}

/// Store bridge transfer
pub fn store_bridge_transfer(env: &Env, transfer: &BridgeTransfer) {
    env.storage()
        .persistent()
        .set(&MonitoringDataKey::BridgeTransfer(transfer.transfer_id), transfer);
}

/// Create new bridge transfer
pub fn create_bridge_transfer(
    env: &Env,
    transfer_id: u64,
    bridge_id: u64,
    source_chain: ChainId,
    destination_chain: ChainId,
    amount: i128,
    fee_paid: i128,
    stellar_asset: Asset,
    user: String,
) -> Result<(), String> {
    if amount <= 0 {
        return Err(String::from_linear(env, "Invalid amount"));
    }

    let transfer = BridgeTransfer {
        transfer_id,
        bridge_id,
        source_chain,
        destination_chain,
        amount,
        fee_paid,
        stellar_asset,
        user,
        status: TransferStatus::Pending,
        validator_signatures: Vec::new(env),
        created_at: env.ledger().timestamp(),
        completed_at: None,
    };

    store_bridge_transfer(env, &transfer);

    env.events().publish(
        (Symbol::new(env, "bridge_transfer_created"), transfer_id),
        (source_chain as u32, destination_chain as u32),
    );

    Ok(())
}

/// Add validator signature
pub fn add_validator_signature(
    env: &Env,
    transfer_id: u64,
    validator: Address,
    signature: String,
) -> Result<(), String> {
    validator.require_auth();
    
    let mut transfer = get_bridge_transfer(env, transfer_id)
        .ok_or_else(|| String::from_linear(env, "Transfer not found"))?;

    // Check for duplicates
    for sig in transfer.validator_signatures.iter() {
        if sig == signature {
            return Err(String::from_linear(env, "Signature already added"));
        }
    }

    transfer.validator_signatures.push_back(signature);

    // Update transfer status when enough signatures received
    if transfer.validator_signatures.len() >= 2 {
        transfer.status = TransferStatus::ValidatorApproved;
    }

    store_bridge_transfer(env, &transfer);

    // Update validator analytics
    update_validator_analytics(
        env,
        validator,
        transfer.bridge_id,
        env.ledger().timestamp(),
        transfer.created_at,
    )?;

    env.events().publish(
        (Symbol::new(env, "validator_signature_added"), transfer_id),
        transfer.validator_signatures.len(),
    );

    Ok(())
}

/// Approve transfer for minting
pub fn approve_transfer_for_minting(env: &Env, transfer_id: u64) -> Result<(), String> {
    let mut transfer = get_bridge_transfer(env, transfer_id)
        .ok_or_else(|| String::from_linear(env, "Transfer not found"))?;

    if transfer.status != TransferStatus::ValidatorApproved {
        return Err(String::from_linear(
            env,
            "Transfer not approved by validators",
        ));
    }

    transfer.status = TransferStatus::Minting;
    store_bridge_transfer(env, &transfer);

    env.events().publish(
        (Symbol::new(env, "transfer_approved_minting"), transfer_id),
        env.ledger().timestamp(),
    );

    Ok(())
}

/// Complete transfer
pub fn complete_transfer(env: &Env, transfer_id: u64) -> Result<(), String> {
    let mut transfer = get_bridge_transfer(env, transfer_id)
        .ok_or_else(|| String::from_linear(env, "Transfer not found"))?;

    transfer.status = TransferStatus::Complete;
    transfer.completed_at = Some(env.ledger().timestamp());
    store_bridge_transfer(env, &transfer);

    // Update bridge analytics
    update_transfer_analytics(env, transfer.bridge_id, &transfer)?;

    env.events().publish(
        (Symbol::new(env, "transfer_complete"), transfer_id),
        env.ledger().timestamp(),
    );

    Ok(())
}

/// ==========================
/// Utility Functions
/// ==========================

/// Get current time from environment
fn current_time(env: &Env) -> u64 {
    env.ledger().timestamp()
}

/// ==========================
/// Tests
/// ==========================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::Env;

    fn setup_env() -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(1000);
        env
    }

    #[test]
    fn test_get_default_ethereum_config() {
        let config = get_default_config(ChainId::Ethereum);
        assert_eq!(config.chain_id, ChainId::Ethereum);
        assert_eq!(config.required_confirmations, ETHEREUM_FINALITY);
        assert_eq!(
            config.verification_method,
            VerificationMethod::EpochFinality
        );
    }

    #[test]
    fn test_get_default_bitcoin_config() {
        let config = get_default_config(ChainId::Bitcoin);
        assert_eq!(config.chain_id, ChainId::Bitcoin);
        assert_eq!(config.required_confirmations, BITCOIN_FINALITY);
        assert_eq!(
            config.verification_method,
            VerificationMethod::Probabilistic
        );
    }

    #[test]
    fn test_get_default_polygon_config() {
        let config = get_default_config(ChainId::Polygon);
        assert_eq!(config.chain_id, ChainId::Polygon);
        assert_eq!(config.required_confirmations, POLYGON_FINALITY);
    }

    #[test]
    fn test_get_default_bsc_config() {
        let config = get_default_config(ChainId::BNB);
        assert_eq!(config.chain_id, ChainId::BNB);
        assert_eq!(config.required_confirmations, BSC_FINALITY);
    }

    #[test]
    fn test_monitor_source_transaction() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        let result = monitor_source_transaction(
            &env,
            1,
            tx_hash,
            ChainId::Ethereum,
            100,
        );

        assert!(result.is_ok());

        let monitored = get_monitored_tx(&env, 1);
        assert!(monitored.is_some());
        let tx = monitored.unwrap();
        assert_eq!(tx.transfer_id, 1);
        assert_eq!(tx.block_number, 100);
        assert_eq!(tx.confirmations, 0);
        assert_eq!(tx.status, MonitoringStatus::Pending);
    }

    #[test]
    fn test_update_confirmation_block_confirmations() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        // Create monitored transaction at block 100
        monitor_source_transaction(
            &env,
            1,
            tx_hash,
            ChainId::Ethereum,
            100,
        ).unwrap();

        // Update at block 132 (32 confirmations)
        let is_finalized = update_transaction_confirmation_count(&env, 1, 132).unwrap();

        assert!(is_finalized);
        let monitored = get_monitored_tx(&env, 1).unwrap();
        assert_eq!(monitored.confirmations, 32);
        assert_eq!(monitored.status, MonitoringStatus::Finalized);
    }

    #[test]
    fn test_update_confirmation_polygon() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        // Polygon requires 128 confirmations
        monitor_source_transaction(
            &env,
            1,
            tx_hash,
            ChainId::Polygon,
            1000,
        ).unwrap();

        // Update at 1100 (100 confirmations, not enough)
        let is_finalized = update_transaction_confirmation_count(&env, 1, 1100).unwrap();
        assert!(!is_finalized);

        // Update at 1128 (128 confirmations, exactly required)
        let is_finalized = update_transaction_confirmation_count(&env, 1, 1128).unwrap();
        assert!(is_finalized);
    }

    #[test]
    fn test_update_confirmation_bitcoin_probabilistic() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        // Bitcoin uses probabilistic finality
        monitor_source_transaction(
            &env,
            1,
            tx_hash,
            ChainId::Bitcoin,
            5000,
        ).unwrap();

        // Update at 5006 (6 confirmations)
        // Bitcoin requires 6 * 2 = 12 for probabilistic finality
        let is_finalized = update_transaction_confirmation_count(&env, 1, 5006).unwrap();
        assert!(!is_finalized);

        // Update at 5012 (12 confirmations)
        let is_finalized = update_transaction_confirmation_count(&env, 1, 5012).unwrap();
        assert!(is_finalized);
    }

    #[test]
    fn test_check_for_reorg_within_depth() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        monitor_source_transaction(
            &env,
            1,
            tx_hash,
            ChainId::Ethereum,
            9900,
        ).unwrap();

        // Check at current_block = 9920 (within reorg depth of 64)
        let is_reorg = check_for_reorg(&env, 1, 9920).unwrap();
        assert!(!is_reorg);
    }

    #[test]
    fn test_handle_reorg_resets_state() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        // Create monitored transaction
        monitor_source_transaction(
            &env,
            1,
            tx_hash,
            ChainId::Ethereum,
            100,
        ).unwrap();

        // Mark as finalized first
        let mut monitored = get_monitored_tx(&env, 1).unwrap();
        mark_as_finalized(&env, &mut monitored).unwrap();

        // Handle reorg
        let result = handle_reorg(&env, 1);
        assert!(result.is_ok());

        // Verify state reset
        let monitored = get_monitored_tx(&env, 1).unwrap();
        assert_eq!(monitored.status, MonitoringStatus::Reorged);
        assert_eq!(monitored.confirmations, 0);
    }

    #[test]
    fn test_create_bridge_transfer() {
        let env = setup_env();
        let user = String::from_linear(&env, "user123");

        let asset = Asset { code: String::from_linear(&env, "XLM"), issuer: None };
        let result = create_bridge_transfer(
            &env,
            1,
            1, // bridge_id
            ChainId::Ethereum,
            ChainId::Polygon,
            1000000,
            100, // fee_paid
            asset,
            user,
        );

        assert!(result.is_ok());

        let transfer = get_bridge_transfer(&env, 1);
        assert!(transfer.is_some());
        let t = transfer.unwrap();
        assert_eq!(t.transfer_id, 1);
        assert_eq!(t.amount, 1000000);
        assert_eq!(t.status, TransferStatus::Pending);
    }

    #[test]
    fn test_add_validator_signature() {
        let env = setup_env();
        let val1 = Address::generate(&env);
        let val2 = Address::generate(&env);
        add_validator_signature(&env, 1, val1, sig1.clone()).unwrap();
        let transfer = get_bridge_transfer(&env, 1).unwrap();
        assert_eq!(transfer.validator_signatures.len(), 1);
        assert_eq!(transfer.status, TransferStatus::Pending); // Not enough

        add_validator_signature(&env, 1, val2, sig2).unwrap();
        let transfer = get_bridge_transfer(&env, 1).unwrap();
        assert_eq!(transfer.validator_signatures.len(), 2);
        assert_eq!(transfer.status, TransferStatus::ValidatorApproved);
    }

    #[test]
    fn test_add_duplicate_signature_fails() {
        let env = setup_env();
        let user = String::from_linear(&env, "user123");

        let sig = String::from_linear(&env, "sig1");
        let val = Address::generate(&env);
        add_validator_signature(&env, 1, val.clone(), sig.clone()).unwrap();

        let result = add_validator_signature(&env, 1, val, sig);
        assert!(result.is_err());
    }

    #[test]
    fn test_approve_transfer_for_minting() {
        let env = setup_env();
        let user = String::from_linear(&env, "user123");

        create_bridge_transfer(
            &env,
            1,
            ChainId::Ethereum,
            ChainId::Polygon,
            1000000,
            user,
        ).unwrap();

        // Try to approve without validator signatures - should fail
        let result = approve_transfer_for_minting(&env, 1);
        assert!(result.is_err());

        // Add signatures
        let sig1 = String::from_linear(&env, "sig1");
        let sig2 = String::from_linear(&env, "sig2");
        let val1 = Address::generate(&env);
        let val2 = Address::generate(&env);
        add_validator_signature(&env, 1, val1, sig1).unwrap();
        add_validator_signature(&env, 1, val2, sig2).unwrap();

        // Now approve should succeed
        let result = approve_transfer_for_minting(&env, 1);
        assert!(result.is_ok());

        let transfer = get_bridge_transfer(&env, 1).unwrap();
        assert_eq!(transfer.status, TransferStatus::Minting);
    }

    #[test]
    fn test_complete_transfer() {
        let env = setup_env();
        let user = String::from_linear(&env, "user123");

        create_bridge_transfer(
            &env,
            1,
            ChainId::Ethereum,
            ChainId::Polygon,
            1000000,
            user,
        ).unwrap();

        let result = complete_transfer(&env, 1);
        assert!(result.is_ok());

        let transfer = get_bridge_transfer(&env, 1).unwrap();
        assert_eq!(transfer.status, TransferStatus::Complete);
    }

    #[test]
    fn test_set_custom_chain_config() {
        let env = setup_env();

        let custom_config = ChainFinalityConfig {
            chain_id: ChainId::Ethereum,
            required_confirmations: 64,  // Custom: more than default 32
            average_block_time: 12,
            reorg_depth_limit: 128,
            verification_method: VerificationMethod::BlockConfirmations,
        };

        set_chain_finality_config(&env, &custom_config);

        let retrieved = get_chain_finality_config(&env, ChainId::Ethereum).unwrap();
        assert_eq!(retrieved.required_confirmations, 64);
    }

    #[test]
    fn test_invalid_transfer_amount() {
        let env = setup_env();
        let user = String::from_linear(&env, "user123");

        let asset = Asset { code: String::from_linear(&env, "XLM"), issuer: None };
        let result = create_bridge_transfer(
            &env,
            1,
            1,
            ChainId::Ethereum,
            ChainId::Polygon,
            0,  // Invalid amount
            0,
            asset,
            user,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_confirmation_progression() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        // Monitor transaction at block 100
        monitor_source_transaction(&env, 1, tx_hash, ChainId::Ethereum, 100).unwrap();

        // Check progression: 0 -> 16 -> 32 confirmations
        for block_num in [116u64, 132u64] {
            let is_finalized = update_transaction_confirmation_count(&env, 1, block_num).unwrap();

            if block_num == 132 {
                assert!(is_finalized);
            } else {
                assert!(!is_finalized);
            }

            let monitored = get_monitored_tx(&env, 1).unwrap();
            assert!(monitored.confirmations > 0);
        }
    }

    #[test]
    fn test_mark_transaction_failed() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        monitor_source_transaction(&env, 1, tx_hash, ChainId::Ethereum, 100).unwrap();

        let result = mark_transaction_failed(&env, 1);
        assert!(result.is_ok());

        let monitored = get_monitored_tx(&env, 1).unwrap();
        assert_eq!(monitored.status, MonitoringStatus::Failed);
    }

    #[test]
    fn test_finalization_with_epoch_finality() {
        let env = setup_env();
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        monitor_source_transaction(&env, 1, tx_hash, ChainId::Ethereum, 100).unwrap();

        // Ethereum finality: 32 blocks
        // EpochFinality method: requires 32/2 = 16+ confirmations
        let is_finalized = update_transaction_confirmation_count(&env, 1, 116).unwrap();
        assert!(is_finalized);
    }

    #[test]
    fn test_full_transfer_workflow() {
        let env = setup_env();
        let user = String::from_linear(&env, "user123");
        let tx_hash = String::from_linear(&env, "0xabcd1234");

        let asset = Asset { code: String::from_linear(&env, "XLM"), issuer: None };
        // Step 1: Create transfer
        create_bridge_transfer(
            &env,
            1,
            1,
            ChainId::Ethereum,
            ChainId::Polygon,
            1000000,
            100,
            asset,
            user,
        ).unwrap();

        // Step 2: Start monitoring
        monitor_source_transaction(&env, 1, tx_hash, ChainId::Ethereum, 100).unwrap();

        // Step 3: Reach finality
        update_transaction_confirmation_count(&env, 1, 132).unwrap();

        // Step 4: Add validator signatures
        let val1 = Address::generate(&env);
        let val2 = Address::generate(&env);
        add_validator_signature(&env, 1, val1, String::from_linear(&env, "sig1")).unwrap();
        add_validator_signature(&env, 1, val2, String::from_linear(&env, "sig2")).unwrap();

        // Step 5: Approve for minting
        approve_transfer_for_minting(&env, 1).unwrap();

        // Step 6: Complete
        complete_transfer(&env, 1).unwrap();

        let transfer = get_bridge_transfer(&env, 1).unwrap();
        assert_eq!(transfer.status, TransferStatus::Complete);
    }
}
