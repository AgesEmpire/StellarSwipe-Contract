# Frontend Integration Guide

This guide shows how to integrate StellarSwipe contracts from a JavaScript/TypeScript frontend using `@stellar/stellar-sdk`.

## Prerequisites

- Node.js 18+
- A funded testnet account
- Contract IDs from `deployments/testnet.json`

Install:

```bash
npm install @stellar/stellar-sdk
```

## Network + Client Setup

```ts
import {
  Address,
  Contract,
  Networks,
  Keypair,
  nativeToScVal,
  rpc,
  scValToNative,
  TransactionBuilder,
  BASE_FEE,
} from "@stellar/stellar-sdk";

const RPC_URL = "https://soroban-testnet.stellar.org";
const NETWORK_PASSPHRASE = Networks.TESTNET;

const server = new rpc.Server(RPC_URL, { allowHttp: false });

export const CONTRACTS = {
  signal_registry: "C_SIGNAL_REGISTRY_ID",
  oracle: "C_ORACLE_ID",
  auto_trade: "C_AUTO_TRADE_ID",
  bridge: "C_BRIDGE_ID",
  governance: "C_GOVERNANCE_ID",
  trade_executor: "C_TRADE_EXECUTOR_ID",
  fee_collector: "C_FEE_COLLECTOR_ID",
  user_portfolio: "C_USER_PORTFOLIO_ID",
} as const;
```

## Wallet Connection (Minimal Pattern)

```ts
type ConnectedWallet = { publicKey: string; signer: Keypair };

export function connectWithSecret(secretKey: string): ConnectedWallet {
  // For production, replace with wallet adapter / hardware wallet flow.
  const signer = Keypair.fromSecret(secretKey);
  return { publicKey: signer.publicKey(), signer };
}
```

## Generic Contract Invocation Helper

```ts
type ContractKey = keyof typeof CONTRACTS;

async function invokeContract(
  contractKey: ContractKey,
  method: string,
  args: unknown[],
  wallet: ConnectedWallet,
  simulateOnly = false
) {
  const account = await server.getAccount(wallet.publicKey);
  const contract = new Contract(CONTRACTS[contractKey]);
  const scArgs = args.map((a) => nativeToScVal(a as never));

  const tx = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(contract.call(method, ...scArgs))
    .setTimeout(60)
    .build();

  const simulated = await server.simulateTransaction(tx);
  if (rpc.Api.isSimulationError(simulated)) {
    throw new Error(`Simulation failed: ${simulated.error}`);
  }
  if (simulateOnly) return simulated;

  const prepared = rpc.assembleTransaction(tx, simulated).build();
  prepared.sign(wallet.signer);
  const sent = await server.sendTransaction(prepared);

  if (sent.status === "PENDING") {
    const finalRes = await server.pollTransaction(sent.hash);
    if (finalRes.status !== "SUCCESS") throw new Error(`Tx failed: ${finalRes.status}`);
    return finalRes.resultXdr ? scValToNative(finalRes.resultXdr) : finalRes;
  }

  if (sent.status !== "SUCCESS") throw new Error(`Tx failed immediately: ${sent.status}`);
  return sent;
}
```

## Error Handling (Common Cases)

```ts
function mapError(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e);

  if (msg.includes("Simulation failed")) return "Contract rejected input or auth";
  if (msg.includes("insufficient balance")) return "Insufficient XLM/tokens for fees or trade";
  if (msg.includes("tx_bad_auth")) return "Wallet signature missing or invalid";
  if (msg.includes("tx_too_late")) return "Transaction expired; rebuild and retry";
  if (msg.includes("resource limit exceeded")) return "Increase fee or reduce operation size";
  if (msg.includes("not found")) return "Invalid contract ID or network mismatch";

  return `Unknown error: ${msg}`;
}
```

## Event Subscription (Testnet)

```ts
async function subscribeContractEvents(contractId: string) {
  // Cursor is a paging token from previous call; persist in storage in real app.
  let cursor: string | undefined;

  setInterval(async () => {
    const events = await server.getEvents({
      startLedger: undefined,
      cursor,
      filters: [{ type: "contract", contractIds: [contractId] }],
      limit: 50,
    });

    for (const ev of events.events) {
      console.log("event", ev.id, ev.topic, ev.value);
    }
    if (events.events.length > 0) cursor = events.events[events.events.length - 1].pagingToken;
  }, 4000);
}
```

## Complete Flow Example

Connect wallet -> submit trade -> handle result -> subscribe to events:

```ts
async function runTradeFlow(secret: string) {
  const wallet = connectWithSecret(secret);

  try {
    // 1) Connect and sanity read
    const health = await invokeContract("signal_registry", "health_check", [], wallet, true);
    console.log("signal_registry health(simulated):", health);

    // 2) Submit a trade-like action (example: execute_copy_trade in trade_executor)
    const result = await invokeContract(
      "trade_executor",
      "execute_copy_trade",
      [
        wallet.publicKey, // user
        1, // signal_id
        "XLM", // asset
        100_0000, // amount
      ],
      wallet
    );
    console.log("trade result:", result);

    // 3) Subscribe to events
    await subscribeContractEvents(CONTRACTS.trade_executor);
  } catch (e) {
    console.error(mapError(e));
  }
}
```

## Contract Function Reference (TypeScript Examples)

Use this call shape for every function:

```ts
await invokeContract("CONTRACT_KEY", "function_name", [/* args */], wallet);
```

### signal_registry

```ts
await invokeContract("signal_registry", "initialize", [admin], wallet);
await invokeContract("signal_registry", "set_trade_executor", [caller, tradeExecutor], wallet);
await invokeContract("signal_registry", "migrate_signals_v1_to_v2", [caller, limit], wallet);
await invokeContract("signal_registry", "set_min_stake", [caller, newAmount], wallet);
await invokeContract("signal_registry", "stake_tokens", [provider, amount], wallet);
await invokeContract("signal_registry", "unstake_tokens", [provider], wallet);
await invokeContract("signal_registry", "set_trade_fee", [caller, newFeeBps], wallet);
await invokeContract("signal_registry", "set_risk_defaults", [caller, maxRiskBps, maxLeverage, maxPositionPct], wallet);
await invokeContract("signal_registry", "set_rate_limit_config", [caller, maxPerMinute, cooldownSecs], wallet);
await invokeContract("signal_registry", "pause_trading", [caller], wallet);
await invokeContract("signal_registry", "unpause_trading", [caller], wallet);
await invokeContract("signal_registry", "pause_fee_collection", [caller], wallet);
await invokeContract("signal_registry", "resume_fee_collection", [caller], wallet);
await invokeContract("signal_registry", "is_fee_collection_paused", [], wallet);
await invokeContract("signal_registry", "pause_category", [caller, category], wallet);
await invokeContract("signal_registry", "unpause_category", [caller, category], wallet);
await invokeContract("signal_registry", "get_pause_states", [], wallet);
await invokeContract("signal_registry", "propose_admin_transfer", [caller, newAdmin], wallet);
await invokeContract("signal_registry", "accept_admin_transfer", [caller], wallet);
await invokeContract("signal_registry", "cancel_admin_transfer", [caller], wallet);
await invokeContract("signal_registry", "set_guardian", [caller, guardian], wallet);
await invokeContract("signal_registry", "revoke_guardian", [caller], wallet);
await invokeContract("signal_registry", "get_guardian", [], wallet);
await invokeContract("signal_registry", "get_admin", [], wallet);
await invokeContract("signal_registry", "schedule", [caller, signalPayload], wallet);
await invokeContract("signal_registry", "trigger_scheduled_publications", [], wallet);
await invokeContract("signal_registry", "cancel_schedule", [caller, scheduleId], wallet);
await invokeContract("signal_registry", "get_config", [], wallet);
await invokeContract("signal_registry", "health_check", [], wallet);
await invokeContract("signal_registry", "set_circuit_breaker_config", [caller, cfg], wallet);
await invokeContract("signal_registry", "get_circuit_breaker_config", [], wallet);
await invokeContract("signal_registry", "get_circuit_breaker_stats", [], wallet);
await invokeContract("signal_registry", "is_paused", [], wallet);
await invokeContract("signal_registry", "get_pause_info", [], wallet);
await invokeContract("signal_registry", "enable_multisig", [caller, signers, threshold], wallet);
await invokeContract("signal_registry", "disable_multisig", [caller], wallet);
await invokeContract("signal_registry", "is_multisig_enabled", [], wallet);
await invokeContract("signal_registry", "get_multisig_signers", [], wallet);
await invokeContract("signal_registry", "get_multisig_threshold", [], wallet);
await invokeContract("signal_registry", "add_multisig_signer", [caller, signer], wallet);
await invokeContract("signal_registry", "remove_multisig_signer", [caller, signer], wallet);
await invokeContract("signal_registry", "create_signal", [provider, payload], wallet);
await invokeContract("signal_registry", "get_signal", [signalId], wallet);
await invokeContract("signal_registry", "update_signal", [provider, signalId, payload], wallet);
await invokeContract("signal_registry", "record_signal_outcome", [caller, signalId, won], wallet);
await invokeContract("signal_registry", "get_provider_reputation_score", [provider], wallet);
await invokeContract("signal_registry", "get_provider_stats", [provider], wallet);
await invokeContract("signal_registry", "create_template", [provider, template], wallet);
await invokeContract("signal_registry", "set_template_public", [provider, templateId, isPublic], wallet);
await invokeContract("signal_registry", "get_template", [templateId], wallet);
await invokeContract("signal_registry", "submit_from_template", [provider, templateId, overrides], wallet);
await invokeContract("signal_registry", "record_trade_execution", [caller, signalId, execution], wallet);
await invokeContract("signal_registry", "get_signal_performance", [signalId], wallet);
await invokeContract("signal_registry", "get_provider_performance", [provider], wallet);
await invokeContract("signal_registry", "get_leaderboard", [limit], wallet);
await invokeContract("signal_registry", "get_top_providers", [limit], wallet);
await invokeContract("signal_registry", "increment_adoption", [signalId], wallet);
await invokeContract("signal_registry", "set_platform_treasury", [caller, treasury], wallet);
await invokeContract("signal_registry", "get_platform_treasury", [], wallet);
await invokeContract("signal_registry", "get_treasury_balance", [asset], wallet);
await invokeContract("signal_registry", "get_all_treasury_balances", [], wallet);
await invokeContract("signal_registry", "calculate_fee_preview", [signalId, amount], wallet);
await invokeContract("signal_registry", "get_active_signals", [limit, cursor], wallet);
await invokeContract("signal_registry", "get_active_signals_archived", [limit, cursor], wallet);
await invokeContract("signal_registry", "follow_provider", [user, provider], wallet);
await invokeContract("signal_registry", "unfollow_provider", [user, provider], wallet);
await invokeContract("signal_registry", "get_followed_providers", [user], wallet);
await invokeContract("signal_registry", "get_follower_count", [provider], wallet);
await invokeContract("signal_registry", "cleanup_expired_signals", [limit], wallet);
await invokeContract("signal_registry", "archive_old_signals", [limit], wallet);
await invokeContract("signal_registry", "get_expired_count", [], wallet);
await invokeContract("signal_registry", "get_pending_expiry_count", [], wallet);
await invokeContract("signal_registry", "get_provider_analytics", [provider], wallet);
await invokeContract("signal_registry", "get_trending_assets", [windowHours], wallet);
await invokeContract("signal_registry", "get_global_analytics", [], wallet);
await invokeContract("signal_registry", "add_tags_to_signal", [provider, signalId, tags], wallet);
await invokeContract("signal_registry", "get_signals_filtered", [query], wallet);
await invokeContract("signal_registry", "get_popular_tags", [limit], wallet);
await invokeContract("signal_registry", "suggest_tags", [rationale], wallet);
await invokeContract("signal_registry", "import_signals_csv", [caller, csvData], wallet);
await invokeContract("signal_registry", "import_signals_json", [caller, jsonData], wallet);
await invokeContract("signal_registry", "get_signal_by_external_id", [externalId], wallet);
await invokeContract("signal_registry", "create_collaborative_signal", [provider, payload], wallet);
await invokeContract("signal_registry", "approve_collaborative_signal", [reviewer, signalId], wallet);
await invokeContract("signal_registry", "get_collaboration_details", [signalId], wallet);
await invokeContract("signal_registry", "is_collaborative_signal", [signalId], wallet);
await invokeContract("signal_registry", "create_combo_signal", [provider, combo], wallet);
await invokeContract("signal_registry", "execute_combo_signal", [caller, comboId], wallet);
await invokeContract("signal_registry", "cancel_combo_signal", [caller, comboId], wallet);
await invokeContract("signal_registry", "get_combo_signal", [comboId], wallet);
await invokeContract("signal_registry", "get_combo_performance", [comboId], wallet);
await invokeContract("signal_registry", "get_combo_executions", [comboId], wallet);
await invokeContract("signal_registry", "create_contest", [caller, contest], wallet);
await invokeContract("signal_registry", "finalize_contest", [contestId], wallet);
await invokeContract("signal_registry", "get_contest", [contestId], wallet);
await invokeContract("signal_registry", "get_active_contests", [], wallet);
await invokeContract("signal_registry", "get_contest_leaderboard", [contestId, limit], wallet);
await invokeContract("signal_registry", "get_provider_prize", [contestId, provider], wallet);
await invokeContract("signal_registry", "update_signal_versioned", [provider, signalId, payload], wallet);
await invokeContract("signal_registry", "get_signal_history", [signalId], wallet);
```

### oracle

```ts
await invokeContract("oracle", "initialize", [admin, baseCurrency], wallet);
await invokeContract("oracle", "health_check", [], wallet);
await invokeContract("oracle", "set_price", [pair, price], wallet);
await invokeContract("oracle", "convert_to_base", [amount, asset], wallet);
await invokeContract("oracle", "get_base_currency", [], wallet);
await invokeContract("oracle", "set_base_currency", [asset], wallet);
await invokeContract("oracle", "add_pair", [pair], wallet);
await invokeContract("oracle", "get_historical_price", [pair, timestamp], wallet);
await invokeContract("oracle", "get_pause_states", [], wallet);
await invokeContract("oracle", "pause_category", [caller, category], wallet);
await invokeContract("oracle", "unpause_category", [caller, category], wallet);
await invokeContract("oracle", "propose_admin_transfer", [caller, newAdmin], wallet);
await invokeContract("oracle", "accept_admin_transfer", [caller], wallet);
await invokeContract("oracle", "cancel_admin_transfer", [caller], wallet);
await invokeContract("oracle", "get_twap_1h", [pair], wallet);
await invokeContract("oracle", "get_twap_24h", [pair], wallet);
await invokeContract("oracle", "get_twap_7d", [pair], wallet);
await invokeContract("oracle", "get_price_deviation", [pair, reference], wallet);
await invokeContract("oracle", "find_optimal_path", [srcAsset, dstAsset], wallet);
await invokeContract("oracle", "calculate_multi_hop_price", [path, amount], wallet);
await invokeContract("oracle", "register_oracle", [admin, oracleAddress], wallet);
await invokeContract("oracle", "submit_price", [oracleAddress, price], wallet);
await invokeContract("oracle", "calculate_consensus", [], wallet);
await invokeContract("oracle", "get_oracle_reputation", [oracleAddress], wallet);
await invokeContract("oracle", "get_oracles", [], wallet);
await invokeContract("oracle", "get_consensus_price", [], wallet);
await invokeContract("oracle", "remove_oracle", [admin, oracleAddress], wallet);
await invokeContract("oracle", "get_price", [pair], wallet);
await invokeContract("oracle", "get_price_with_confidence", [pair], wallet);
await invokeContract("oracle", "add_price_source", [admin, source], wallet);
await invokeContract("oracle", "submit_pair_price", [oracleAddress, pair, price], wallet);
await invokeContract("oracle", "refresh_from_sdex", [pair], wallet);
await invokeContract("oracle", "update_with_external_data", [admin, payload], wallet);
await invokeContract("oracle", "get_safe_price", [pair], wallet);
```

### auto_trade

```ts
await invokeContract("auto_trade", "initialize", [admin], wallet);
await invokeContract("auto_trade", "pause_category", [caller, category], wallet);
await invokeContract("auto_trade", "unpause_category", [caller, category], wallet);
await invokeContract("auto_trade", "set_guardian", [caller, guardian], wallet);
await invokeContract("auto_trade", "revoke_guardian", [caller], wallet);
await invokeContract("auto_trade", "propose_admin_transfer", [caller, newAdmin], wallet);
await invokeContract("auto_trade", "accept_admin_transfer", [caller], wallet);
await invokeContract("auto_trade", "cancel_admin_transfer", [caller], wallet);
await invokeContract("auto_trade", "get_guardian", [], wallet);
await invokeContract("auto_trade", "get_pause_states", [], wallet);
await invokeContract("auto_trade", "set_oracle_address", [caller, oracle], wallet);
await invokeContract("auto_trade", "get_oracle_address", [], wallet);
await invokeContract("auto_trade", "override_oracle_circuit_breaker", [caller, enabled], wallet);
await invokeContract("auto_trade", "get_oracle_circuit_breaker_state", [], wallet);
await invokeContract("auto_trade", "add_oracle", [caller, oracle], wallet);
await invokeContract("auto_trade", "remove_oracle", [caller, oracle], wallet);
await invokeContract("auto_trade", "get_oracle_whitelist", [], wallet);
await invokeContract("auto_trade", "push_price_update", [oracle, asset, price], wallet);
await invokeContract("auto_trade", "set_circuit_breaker_config", [caller, cfg], wallet);
await invokeContract("auto_trade", "execute_trade", [user, signalId, asset, amount], wallet);
await invokeContract("auto_trade", "open_position", [user, signalId, size, entry], wallet);
await invokeContract("auto_trade", "close_position", [user, positionId], wallet);
await invokeContract("auto_trade", "get_all_positions", [user], wallet);
await invokeContract("auto_trade", "get_open_positions", [user], wallet);
await invokeContract("auto_trade", "get_closed_positions", [user], wallet);
await invokeContract("auto_trade", "get_trade", [user, signalId], wallet);
await invokeContract("auto_trade", "upsert_routing_venue", [signalId, venue], wallet);
await invokeContract("auto_trade", "get_routing_venues", [signalId], wallet);
await invokeContract("auto_trade", "preview_smart_route", [signalId, amount], wallet);
await invokeContract("auto_trade", "get_risk_config", [user], wallet);
await invokeContract("auto_trade", "set_risk_config", [user, config], wallet);
await invokeContract("auto_trade", "get_user_positions", [user], wallet);
await invokeContract("auto_trade", "get_trade_history_legacy", [user], wallet);
await invokeContract("auto_trade", "get_trade_history", [user, page, pageSize], wallet);
await invokeContract("auto_trade", "get_portfolio", [user], wallet);
await invokeContract("auto_trade", "set_risk_parity_config", [user, config], wallet);
await invokeContract("auto_trade", "get_risk_parity_config", [user], wallet);
await invokeContract("auto_trade", "preview_risk_parity_rebalance", [user], wallet);
await invokeContract("auto_trade", "trigger_risk_parity_rebalance", [user], wallet);
await invokeContract("auto_trade", "record_asset_price", [assetId, price], wallet);
await invokeContract("auto_trade", "process_price_update", [assetId], wallet);
await invokeContract("auto_trade", "get_trailing_stop_price", [user, assetId], wallet);
await invokeContract("auto_trade", "grant_authorization", [user, delegate], wallet);
await invokeContract("auto_trade", "revoke_authorization", [user], wallet);
await invokeContract("auto_trade", "init_rate_limit_admin", [admin], wallet);
await invokeContract("auto_trade", "set_rate_limits", [caller, limits], wallet);
await invokeContract("auto_trade", "add_to_whitelist", [user], wallet);
await invokeContract("auto_trade", "remove_from_whitelist", [user], wallet);
await invokeContract("auto_trade", "record_violation", [user, reason], wallet);
await invokeContract("auto_trade", "adjust_rate_limits", [], wallet);
await invokeContract("auto_trade", "get_rate_limits", [], wallet);
await invokeContract("auto_trade", "get_user_rate_history", [user], wallet);
await invokeContract("auto_trade", "is_whitelisted", [user], wallet);
await invokeContract("auto_trade", "get_auth_config", [user], wallet);
await invokeContract("auto_trade", "create_dca", [user, strategy], wallet);
await invokeContract("auto_trade", "execute_due_dca", [], wallet);
await invokeContract("auto_trade", "execute_dca_purchase", [strategyId], wallet);
await invokeContract("auto_trade", "pause_dca", [user, strategyId], wallet);
await invokeContract("auto_trade", "resume_dca", [user, strategyId], wallet);
await invokeContract("auto_trade", "update_dca", [user, strategyId, patch], wallet);
await invokeContract("auto_trade", "handle_missed_dca", [strategyId], wallet);
await invokeContract("auto_trade", "get_dca_strategy", [strategyId], wallet);
await invokeContract("auto_trade", "analyze_dca", [strategyId], wallet);
await invokeContract("auto_trade", "create_mean_reversion", [user, cfg], wallet);
await invokeContract("auto_trade", "get_mean_reversion", [strategyId], wallet);
await invokeContract("auto_trade", "check_mr_signals", [strategyId], wallet);
await invokeContract("auto_trade", "execute_mr_trade", [strategyId], wallet);
await invokeContract("auto_trade", "check_mr_exits", [strategyId], wallet);
await invokeContract("auto_trade", "adjust_mr_params", [user, strategyId, patch], wallet);
await invokeContract("auto_trade", "disable_mean_reversion", [user, strategyId], wallet);
await invokeContract("auto_trade", "enable_mean_reversion", [user, strategyId], wallet);
await invokeContract("auto_trade", "set_stat_arb_price_history", [assetId, prices], wallet);
await invokeContract("auto_trade", "get_stat_arb_price_history", [assetId], wallet);
await invokeContract("auto_trade", "configure_stat_arb_strategy", [user, cfg], wallet);
await invokeContract("auto_trade", "get_stat_arb_strategy", [strategyId], wallet);
await invokeContract("auto_trade", "test_stat_arb_cointegration", [assetA, assetB], wallet);
await invokeContract("auto_trade", "check_stat_arb_signal", [strategyId], wallet);
await invokeContract("auto_trade", "execute_stat_arb_trade", [strategyId], wallet);
await invokeContract("auto_trade", "get_active_stat_arb_portfolio", [user], wallet);
await invokeContract("auto_trade", "rebalance_stat_arb_portfolio", [user], wallet);
await invokeContract("auto_trade", "check_stat_arb_exit", [user], wallet);
await invokeContract("auto_trade", "close_stat_arb_portfolio", [user], wallet);
await invokeContract("auto_trade", "configure_insurance", [user, cfg], wallet);
await invokeContract("auto_trade", "get_portfolio_drawdown", [user], wallet);
await invokeContract("auto_trade", "apply_hedge_if_needed", [user], wallet);
await invokeContract("auto_trade", "rebalance_hedges", [user], wallet);
await invokeContract("auto_trade", "remove_hedges_if_recovered", [user], wallet);
await invokeContract("auto_trade", "get_insurance_config", [user], wallet);
await invokeContract("auto_trade", "create_exit_strategy", [user, cfg], wallet);
await invokeContract("auto_trade", "create_exit_strategy_conservative", [user], wallet);
await invokeContract("auto_trade", "create_exit_strategy_balanced", [user], wallet);
await invokeContract("auto_trade", "create_exit_strategy_aggressive", [user], wallet);
await invokeContract("auto_trade", "check_and_execute_exits", [user], wallet);
await invokeContract("auto_trade", "get_exit_strategy", [strategyId], wallet);
await invokeContract("auto_trade", "get_user_exit_strategies", [user], wallet);
await invokeContract("auto_trade", "adjust_exit_position", [user, strategyId, patch], wallet);
await invokeContract("auto_trade", "init_grid", [user, cfg], wallet);
await invokeContract("auto_trade", "place_grid_orders", [strategyId], wallet);
await invokeContract("auto_trade", "grid_order_filled", [strategyId, orderId], wallet);
await invokeContract("auto_trade", "adjust_grid", [strategyId], wallet);
```

### bridge

```ts
await invokeContract("bridge", "initialize", [admin, config], wallet);
await invokeContract("bridge", "register_wrapped_asset", [admin, wrappedAsset], wallet);
await invokeContract("bridge", "initiate_lock_mint", [user, amount, srcChain, dstChain], wallet);
await invokeContract("bridge", "approve_lock_mint", [validator, transferId], wallet);
await invokeContract("bridge", "execute_lock_mint", [transferId], wallet);
await invokeContract("bridge", "initiate_burn_unlock", [user, amount, wrappedAsset, dstChain], wallet);
await invokeContract("bridge", "approve_burn_unlock", [validator, transferId], wallet);
await invokeContract("bridge", "execute_burn_unlock", [transferId], wallet);
await invokeContract("bridge", "get_transfer", [transferId], wallet);
await invokeContract("bridge", "get_bridge_config", [], wallet);
await invokeContract("bridge", "get_wrapped_balance", [user, wrappedAsset], wallet);
await invokeContract("bridge", "create_liquidity_pool", [admin, poolCfg], wallet);
await invokeContract("bridge", "add_bridge_liquidity", [provider, poolId, amountA, amountB], wallet);
await invokeContract("bridge", "remove_bridge_liquidity", [provider, poolId, lpAmount], wallet);
await invokeContract("bridge", "swap_bridge_assets", [user, poolId, assetIn, amountIn], wallet);
await invokeContract("bridge", "get_pool", [poolId], wallet);
await invokeContract("bridge", "get_liquidity_position", [provider, poolId], wallet);
await invokeContract("bridge", "get_pool_health", [poolId], wallet);
await invokeContract("bridge", "health_check", [], wallet);
```

### governance

```ts
await invokeContract("governance", "initialize", [admin, tokenConfig], wallet);
await invokeContract("governance", "health_check", [], wallet);
await invokeContract("governance", "set_contract_paused", [admin, paused], wallet);
await invokeContract("governance", "get_metadata", [], wallet);
await invokeContract("governance", "total_supply", [], wallet);
await invokeContract("governance", "circulating_supply", [], wallet);
await invokeContract("governance", "balance", [holder], wallet);
await invokeContract("governance", "staked_balance", [holder], wallet);
await invokeContract("governance", "voting_power", [holder], wallet);
await invokeContract("governance", "governance_config", [], wallet);
await invokeContract("governance", "configure_governance", [admin, cfg], wallet);
await invokeContract("governance", "create_proposal", [proposer, proposal], wallet);
await invokeContract("governance", "proposal", [proposalId], wallet);
await invokeContract("governance", "proposals", [], wallet);
await invokeContract("governance", "cast_vote", [voter, proposalId, support], wallet);
await invokeContract("governance", "finalize_proposal", [proposalId], wallet);
await invokeContract("governance", "execute_proposal", [proposalId], wallet);
await invokeContract("governance", "cancel_proposal", [caller, proposalId], wallet);
await invokeContract("governance", "proposal_statistics", [], wallet);
await invokeContract("governance", "delegate_voting_power", [delegator, delegatee], wallet);
await invokeContract("governance", "undelegate_voting_power", [delegator], wallet);
await invokeContract("governance", "effective_voting_power", [user], wallet);
await invokeContract("governance", "initialize_timelock", [admin, delaySecs], wallet);
await invokeContract("governance", "queue_action", [proposalId], wallet);
await invokeContract("governance", "execute_queued_action", [queueId], wallet);
await invokeContract("governance", "cancel_queued_action", [admin, queueId], wallet);
await invokeContract("governance", "update_timelock_delay", [admin, delaySecs], wallet);
await invokeContract("governance", "emergency_execute", [admin, action], wallet);
await invokeContract("governance", "timelock_analytics", [], wallet);
await invokeContract("governance", "extend_execution_window", [admin, proposalId, extraSecs], wallet);
await invokeContract("governance", "execute_multiple_actions", [admin, actions], wallet);
await invokeContract("governance", "governance_reputation", [user], wallet);
await invokeContract("governance", "calculate_reputation_score", [user], wallet);
await invokeContract("governance", "cast_reputation_weighted_vote", [voter, proposalId, support], wallet);
await invokeContract("governance", "reputation_leaderboard", [limit], wallet);
await invokeContract("governance", "distribute_reputation_rewards", [admin, amount], wallet);
await invokeContract("governance", "create_conviction_pool", [admin, cfg], wallet);
await invokeContract("governance", "conviction_pool", [poolId], wallet);
await invokeContract("governance", "create_conviction_proposal", [proposer, poolId, proposal], wallet);
await invokeContract("governance", "vote_conviction", [voter, proposalId, amount], wallet);
await invokeContract("governance", "update_proposal_conviction", [proposalId], wallet);
await invokeContract("governance", "execute_conviction_funding", [proposalId], wallet);
await invokeContract("governance", "change_conviction_vote", [voter, proposalId, newAmount], wallet);
await invokeContract("governance", "refill_conviction_pool", [poolId], wallet);
await invokeContract("governance", "withdraw_conviction_vote", [voter, proposalId], wallet);
await invokeContract("governance", "analyze_conviction_proposal", [proposalId], wallet);
await invokeContract("governance", "conviction_growth_curve", [proposalId], wallet);
await invokeContract("governance", "distribution", [], wallet);
await invokeContract("governance", "create_vesting_schedule", [admin, beneficiary, schedule], wallet);
await invokeContract("governance", "get_vesting_schedule", [beneficiary], wallet);
await invokeContract("governance", "releasable_vested_amount", [beneficiary], wallet);
await invokeContract("governance", "release_vested_tokens", [beneficiary], wallet);
await invokeContract("governance", "stake", [user, amount], wallet);
await invokeContract("governance", "unstake", [user, amount], wallet);
await invokeContract("governance", "set_vote_lock", [admin, user, untilTs], wallet);
await invokeContract("governance", "accrue_liquidity_rewards", [admin], wallet);
await invokeContract("governance", "claim_liquidity_rewards", [beneficiary], wallet);
await invokeContract("governance", "pending_rewards", [beneficiary], wallet);
await invokeContract("governance", "set_liquidity_mining_config", [admin, cfg], wallet);
await invokeContract("governance", "analytics", [topN], wallet);
await invokeContract("governance", "treasury", [], wallet);
await invokeContract("governance", "set_treasury_asset", [admin, asset], wallet);
await invokeContract("governance", "create_budget", [admin, budget], wallet);
await invokeContract("governance", "execute_treasury_spend", [admin, spendId], wallet);
await invokeContract("governance", "create_recurring_payment", [admin, payment], wallet);
await invokeContract("governance", "process_recurring_payments", [admin], wallet);
await invokeContract("governance", "treasury_report", [], wallet);
await invokeContract("governance", "committees", [], wallet);
await invokeContract("governance", "committee", [committeeId], wallet);
await invokeContract("governance", "create_committee", [admin, committee], wallet);
await invokeContract("governance", "propose_committee_decision", [member, committeeId, decision], wallet);
await invokeContract("governance", "vote_on_committee_decision", [member, decisionId, support], wallet);
await invokeContract("governance", "execute_committee_decision", [committeeId, decisionId], wallet);
await invokeContract("governance", "start_committee_election", [admin, electionCfg], wallet);
await invokeContract("governance", "committee_election", [electionId], wallet);
await invokeContract("governance", "nominate_for_committee", [nominator, electionId, nominee], wallet);
await invokeContract("governance", "vote_in_committee_election", [voter, electionId, nominee], wallet);
await invokeContract("governance", "finalize_committee_election", [electionId], wallet);
await invokeContract("governance", "set_committee_approval_rating", [admin, committeeId, rating], wallet);
await invokeContract("governance", "committee_report", [committeeId], wallet);
await invokeContract("governance", "override_committee_decision", [admin, committeeId, decisionId], wallet);
await invokeContract("governance", "dissolve_committee", [admin, committeeId], wallet);
await invokeContract("governance", "request_cross_committee_approval", [requester, req], wallet);
await invokeContract("governance", "approve_cross_committee_request", [approver, requestId], wallet);
await invokeContract("governance", "cross_committee_request", [requestId], wallet);
await invokeContract("governance", "set_rebalance_target", [admin, target], wallet);
await invokeContract("governance", "rebalance_treasury", [admin], wallet);
```

### trade_executor

```ts
await invokeContract("trade_executor", "initialize", [admin], wallet);
await invokeContract("trade_executor", "set_user_portfolio", [portfolio], wallet);
await invokeContract("trade_executor", "get_user_portfolio", [], wallet);
await invokeContract("trade_executor", "set_copy_trade_estimated_fee", [fee], wallet);
await invokeContract("trade_executor", "get_copy_trade_estimated_fee", [], wallet);
await invokeContract("trade_executor", "set_position_limit_exempt", [user, exempt], wallet);
await invokeContract("trade_executor", "is_position_limit_exempt", [user], wallet);
await invokeContract("trade_executor", "set_oracle", [oracle], wallet);
await invokeContract("trade_executor", "set_stop_loss_portfolio", [portfolio], wallet);
await invokeContract("trade_executor", "set_stop_loss_price", [user, tradeId, stopLoss], wallet);
await invokeContract("trade_executor", "check_and_trigger_stop_loss", [user, tradeId], wallet);
await invokeContract("trade_executor", "set_take_profit_price", [user, tradeId, takeProfit], wallet);
await invokeContract("trade_executor", "check_and_trigger_take_profit", [user, tradeId], wallet);
await invokeContract("trade_executor", "get_insufficient_balance_detail", [user, amount], wallet);
await invokeContract("trade_executor", "execute_copy_trade", [user, signalId, asset, amount], wallet);
await invokeContract("trade_executor", "set_sdex_router", [router], wallet);
await invokeContract("trade_executor", "get_sdex_router", [], wallet);
await invokeContract("trade_executor", "swap", [user, path, amount], wallet);
await invokeContract("trade_executor", "swap_with_slippage", [user, path, amount, maxSlippageBps], wallet);
await invokeContract("trade_executor", "cancel_copy_trade", [user, signalId], wallet);
```

### fee_collector

```ts
await invokeContract("fee_collector", "initialize", [admin], wallet);
await invokeContract("fee_collector", "set_oracle_contract", [oracleContract], wallet);
await invokeContract("fee_collector", "fee_rate_for_user", [user], wallet);
await invokeContract("fee_collector", "monthly_trade_volume", [user], wallet);
await invokeContract("fee_collector", "treasury_balance", [token], wallet);
await invokeContract("fee_collector", "queue_withdrawal", [admin, token, amount, releaseTs], wallet);
await invokeContract("fee_collector", "withdraw_treasury_fees", [admin, token, amount], wallet);
await invokeContract("fee_collector", "fee_rate", [], wallet);
await invokeContract("fee_collector", "set_fee_rate", [newRateBps], wallet);
await invokeContract("fee_collector", "collect_fee", [user, token, amount], wallet);
await invokeContract("fee_collector", "claim_fees", [provider, token], wallet);
```

### user_portfolio

```ts
await invokeContract("user_portfolio", "initialize", [admin, oracle], wallet);
await invokeContract("user_portfolio", "set_oracle", [oracle], wallet);
await invokeContract("user_portfolio", "open_position", [user, entryPrice, amount], wallet);
await invokeContract("user_portfolio", "close_position", [user, positionId, realizedPnl], wallet);
await invokeContract("user_portfolio", "get_pnl", [user], wallet);
```

## Validation Checklist

- [ ] Event subscription tested on testnet for at least one deployed contract
- [ ] Frontend team confirmed invocation examples are sufficient
- [ ] Error mapping covers wallet auth, simulation, balance, timeout, and network errors
