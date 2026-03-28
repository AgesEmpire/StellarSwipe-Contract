#![no_std]

use soroban_sdk::{contract, contractimpl, Env};
use stellar_swipe_common::HealthStatus;

pub mod monitoring;
pub mod governance;
pub mod analytics;
pub mod fees;
pub mod messaging;

pub use monitoring::{
    ChainFinalityConfig, ChainId, MonitoredTransaction, MonitoringStatus, VerificationMethod,
    BridgeTransfer, TransferStatus,
    monitor_source_transaction, get_monitored_tx, check_for_reorg, handle_reorg,
    update_transaction_confirmation_count, mark_transaction_failed, create_bridge_transfer,
    add_validator_signature, approve_transfer_for_minting, complete_transfer,
    get_chain_finality_config, set_chain_finality_config,
};

pub use governance::{
    BridgeGovernance, GovernanceProposal, ProposalType, ProposalStatus,
    BridgeSecurityConfig, Bridge, BridgeStatus,
    initialize_bridge_governance, initialize_bridge,
    create_bridge_proposal, sign_bridge_proposal, execute_bridge_proposal,
    emergency_execute_proposal, cancel_proposal,
    get_proposal_details, get_bridge_proposals, get_pending_proposals,
    rotate_bridge_signers, add_signer, remove_signer,
    get_bridge_status, get_bridge_validators, get_governance_signers,
    get_required_signatures, is_signer, is_validator,
};

pub use analytics::{
    BridgeAnalytics, ValidatorAnalytics, TimeSeries, DataPoint, TimeInterval,
    VolumeStats, TimePeriod, AnalyticsMetric, Trend, TrendAnalysis,
    get_bridge_analytics, get_validator_analytics, get_bridge_volume_stats,
    calculate_bridge_health_score, compare_bridge_performance, analyze_volume_trend,
};

pub use fees::{
    BridgeFeeConfig, BridgeFeeStats,
    set_bridge_fee_config, get_bridge_fee_config,
    get_bridge_fee_stats,
    set_bridge_treasury, get_bridge_treasury,
    calculate_bridge_fee, collect_bridge_fee,
    distribute_validator_rewards, allocate_to_treasury,
    adjust_bridge_fees_dynamically, calculate_bridge_utilization,
    refund_bridge_fee,
};

pub use messaging::{
    CrossChainMessage, MessageStatus,
    MAX_MESSAGE_SIZE, MESSAGE_TIMEOUT,
    register_bridge_for_chain,
    send_cross_chain_message,
    relay_message_to_target_chain,
    confirm_message_delivery,
    receive_message_callback,
    mark_message_failed,
    retry_failed_message,
    expire_timed_out_message,
    get_cross_chain_message,
};

#[contract]
pub struct BridgeContract;

#[contractimpl]
impl BridgeContract {
    /// Read-only health for ops / frontends; no auth, no storage writes.
    pub fn health_check(env: Env) -> HealthStatus {
        crate::governance::bridge_health_check(&env)
    }
}

#[cfg(test)]
mod test_health;
