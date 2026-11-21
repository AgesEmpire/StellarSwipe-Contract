#!/usr/bin/env tsx
/**
 * monitor.ts — Contract event monitoring and alerting for StellarSwipe.
 *
 * Polls Soroban contract events every 5 minutes and sends Slack alerts for:
 *   - Oracle heartbeat failures (no oracle update within threshold)
 *   - Elevated error rate (errors / total calls > threshold)
 *   - Unusual fee spikes (fee > spike threshold)
 *   - Contract pause events
 *
 * Usage:
 *   SLACK_WEBHOOK_URL=https://hooks.slack.com/... npx tsx scripts/monitor.ts
 *
 * Required env:
 *   STELLAR_SOURCE_ACCOUNT   Signing identity / secret key
 *
 * Optional env:
 *   STELLAR_NETWORK                  default: testnet
 *   STELLAR_RPC_URL                  default: https://soroban-testnet.stellar.org
 *   STELLAR_NETWORK_PASSPHRASE       default: Test SDF Network ; September 2015
 *   DEPLOY_STATE                     path to deployment state JSON
 *   SLACK_WEBHOOK_URL                Slack incoming webhook URL
 *   POLL_INTERVAL_MS                 default: 300000 (5 minutes)
 *   ORACLE_HEARTBEAT_THRESHOLD_MS    default: 600000 (10 minutes — alert if no update)
 *   ERROR_RATE_THRESHOLD             default: 0.05 (5%)
 *   FEE_SPIKE_THRESHOLD_BPS          default: 450 (basis points)
 *
 * Constraint: monitoring script does not require any contract modifications.
 */

import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

// ── Config ────────────────────────────────────────────────────────────────────

const NETWORK = process.env.STELLAR_NETWORK ?? "testnet";
const RPC_URL =
  process.env.STELLAR_RPC_URL ??
  (NETWORK === "mainnet"
    ? "https://mainnet.sorobanrpc.com"
    : "https://soroban-testnet.stellar.org");
const NETWORK_PASSPHRASE =
  process.env.STELLAR_NETWORK_PASSPHRASE ??
  (NETWORK === "mainnet"
    ? "Public Global Stellar Network ; September 2015"
    : "Test SDF Network ; September 2015");

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(SCRIPT_DIR, "..");
const DEPLOY_STATE =
  process.env.DEPLOY_STATE ?? path.join(ROOT, "deployments", `${NETWORK}.json`);

const SLACK_WEBHOOK_URL = process.env.SLACK_WEBHOOK_URL ?? "";
const POLL_INTERVAL_MS = parseInt(process.env.POLL_INTERVAL_MS ?? "300000", 10);
const ORACLE_HEARTBEAT_THRESHOLD_MS = parseInt(
  process.env.ORACLE_HEARTBEAT_THRESHOLD_MS ?? "600000",
  10
);
const ERROR_RATE_THRESHOLD = parseFloat(process.env.ERROR_RATE_THRESHOLD ?? "0.05");
const FEE_SPIKE_THRESHOLD_BPS = parseInt(process.env.FEE_SPIKE_THRESHOLD_BPS ?? "450", 10);

// ── Types ─────────────────────────────────────────────────────────────────────

interface ContractState {
  contracts: Record<string, { contract_id?: string }>;
}

interface MonitorState {
  lastOracleUpdateMs: number;
  totalCalls: number;
  errorCalls: number;
}

interface SorobanEvent {
  type: string;
  contractId?: string;
  topic?: unknown[];
  value?: unknown;
}

// ── State ─────────────────────────────────────────────────────────────────────

const monitorState: MonitorState = {
  lastOracleUpdateMs: Date.now(),
  totalCalls: 0,
  errorCalls: 0,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

function log(msg: string): void {
  console.log(`[${new Date().toISOString()}] ${msg}`);
}

function loadDeployState(): ContractState | null {
  if (!fs.existsSync(DEPLOY_STATE)) {
    log(`WARN: deploy state not found at ${DEPLOY_STATE} — skipping contract-specific checks`);
    return null;
  }
  return JSON.parse(fs.readFileSync(DEPLOY_STATE, "utf8")) as ContractState;
}

function getContractId(state: ContractState, logical: string): string | undefined {
  return state.contracts[logical]?.contract_id;
}

// ── Alerting ──────────────────────────────────────────────────────────────────

async function sendSlackAlert(message: string): Promise<void> {
  if (!SLACK_WEBHOOK_URL) {
    log(`[ALERT] (no Slack webhook configured) ${message}`);
    return;
  }

  const payload = {
    text: `🚨 *StellarSwipe Monitor Alert* (${NETWORK})\n${message}`,
  };

  try {
    const response = await fetch(SLACK_WEBHOOK_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!response.ok) {
      log(`WARN: Slack webhook returned HTTP ${response.status}`);
    } else {
      log(`[ALERT SENT] ${message}`);
    }
  } catch (err) {
    log(`WARN: Failed to send Slack alert: ${err instanceof Error ? err.message : err}`);
  }
}

// ── Event fetching ────────────────────────────────────────────────────────────

/**
 * Fetch recent contract events from the Soroban RPC.
 * Uses the JSON-RPC `getEvents` method directly to avoid requiring stellar CLI.
 */
async function fetchRecentEvents(
  contractIds: string[],
  startLedger: number
): Promise<SorobanEvent[]> {
  if (contractIds.length === 0) return [];

  const body = {
    jsonrpc: "2.0",
    id: 1,
    method: "getEvents",
    params: {
      startLedger,
      filters: contractIds.map((id) => ({
        type: "contract",
        contractIds: [id],
      })),
      pagination: { limit: 200 },
    },
  };

  const response = await fetch(RPC_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    throw new Error(`RPC request failed: HTTP ${response.status}`);
  }

  const json = (await response.json()) as {
    result?: { events?: SorobanEvent[] };
    error?: { message: string };
  };

  if (json.error) {
    throw new Error(`RPC error: ${json.error.message}`);
  }

  return json.result?.events ?? [];
}

/**
 * Fetch the latest ledger number from the RPC.
 */
async function getLatestLedger(): Promise<number> {
  const body = {
    jsonrpc: "2.0",
    id: 1,
    method: "getLatestLedger",
    params: {},
  };

  const response = await fetch(RPC_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    throw new Error(`getLatestLedger failed: HTTP ${response.status}`);
  }

  const json = (await response.json()) as {
    result?: { sequence?: number };
    error?: { message: string };
  };

  if (json.error) throw new Error(`RPC error: ${json.error.message}`);
  return json.result?.sequence ?? 0;
}

// ── Checks ────────────────────────────────────────────────────────────────────

/**
 * Check oracle heartbeat — alert if no oracle update event seen within threshold.
 */
async function checkOracleHeartbeat(events: SorobanEvent[]): Promise<void> {
  // Look for oracle price update events (topic contains "price_updated" or "oracle")
  const oracleEvents = events.filter((e) => {
    const topicStr = JSON.stringify(e.topic ?? "").toLowerCase();
    return topicStr.includes("price") || topicStr.includes("oracle") || topicStr.includes("feed");
  });

  if (oracleEvents.length > 0) {
    monitorState.lastOracleUpdateMs = Date.now();
    log(`[oracle] Heartbeat OK — ${oracleEvents.length} oracle event(s) in this window`);
    return;
  }

  const silenceMs = Date.now() - monitorState.lastOracleUpdateMs;
  if (silenceMs > ORACLE_HEARTBEAT_THRESHOLD_MS) {
    const silenceMin = Math.round(silenceMs / 60000);
    await sendSlackAlert(
      `⚠️ *Oracle Heartbeat Failure*\n` +
      `No oracle price update detected in the last ${silenceMin} minutes.\n` +
      `Threshold: ${ORACLE_HEARTBEAT_THRESHOLD_MS / 60000} minutes.\n` +
      `Action: Check oracle contract and price feed source.`
    );
  } else {
    log(`[oracle] No update this window — silence ${Math.round(silenceMs / 1000)}s (within threshold)`);
  }
}

/**
 * Check error rate — alert if errors / total calls exceeds threshold.
 */
async function checkErrorRate(events: SorobanEvent[]): Promise<void> {
  const errorEvents = events.filter((e) => {
    const topicStr = JSON.stringify(e.topic ?? "").toLowerCase();
    const valueStr = JSON.stringify(e.value ?? "").toLowerCase();
    return (
      topicStr.includes("error") ||
      topicStr.includes("fail") ||
      valueStr.includes("error") ||
      valueStr.includes("panic")
    );
  });

  monitorState.totalCalls += events.length;
  monitorState.errorCalls += errorEvents.length;

  if (monitorState.totalCalls === 0) return;

  const rate = monitorState.errorCalls / monitorState.totalCalls;
  log(
    `[errors] ${errorEvents.length} error event(s) this window — ` +
    `cumulative rate: ${(rate * 100).toFixed(2)}% (${monitorState.errorCalls}/${monitorState.totalCalls})`
  );

  if (rate > ERROR_RATE_THRESHOLD) {
    await sendSlackAlert(
      `⚠️ *Elevated Error Rate*\n` +
      `Error rate: ${(rate * 100).toFixed(2)}% (threshold: ${ERROR_RATE_THRESHOLD * 100}%)\n` +
      `Errors: ${monitorState.errorCalls} / Total calls: ${monitorState.totalCalls}\n` +
      `Action: Review contract logs and recent transactions.`
    );
  }
}

/**
 * Check for fee spike events.
 */
async function checkFeeSpikes(events: SorobanEvent[]): Promise<void> {
  for (const event of events) {
    const topicStr = JSON.stringify(event.topic ?? "").toLowerCase();
    if (!topicStr.includes("fee")) continue;

    // Try to extract fee value from event value
    let feeBps: number | undefined;
    try {
      const val = event.value as { fee_bps?: number; fee?: number } | undefined;
      feeBps = val?.fee_bps ?? val?.fee;
    } catch {
      // ignore parse errors
    }

    if (feeBps !== undefined && feeBps > FEE_SPIKE_THRESHOLD_BPS) {
      await sendSlackAlert(
        `⚠️ *Unusual Fee Spike Detected*\n` +
        `Fee: ${feeBps} bps (threshold: ${FEE_SPIKE_THRESHOLD_BPS} bps)\n` +
        `Contract: ${event.contractId ?? "unknown"}\n` +
        `Action: Verify fee configuration and recent governance proposals.`
      );
    }
  }
}

/**
 * Check for contract pause events.
 */
async function checkPauseEvents(events: SorobanEvent[]): Promise<void> {
  const pauseEvents = events.filter((e) => {
    const topicStr = JSON.stringify(e.topic ?? "").toLowerCase();
    return topicStr.includes("pause") || topicStr.includes("emergency") || topicStr.includes("halt");
  });

  for (const event of pauseEvents) {
    await sendSlackAlert(
      `🛑 *Contract Pause / Emergency Event Detected*\n` +
      `Contract: ${event.contractId ?? "unknown"}\n` +
      `Topic: ${JSON.stringify(event.topic)}\n` +
      `Value: ${JSON.stringify(event.value)}\n` +
      `Action: Investigate immediately — contract may be paused.`
    );
  }
}

// ── Poll cycle ────────────────────────────────────────────────────────────────

let lastLedger = 0;

async function poll(): Promise<void> {
  log(`[POLL] Starting poll cycle (network: ${NETWORK}, rpc: ${RPC_URL})`);

  const deployState = loadDeployState();
  const contractIds: string[] = [];

  if (deployState) {
    for (const logical of [
      "stake_vault",
      "signal_registry",
      "fee_collector",
      "user_portfolio",
      "trade_executor",
    ]) {
      const id = getContractId(deployState, logical);
      if (id) contractIds.push(id);
    }
    log(`[POLL] Monitoring ${contractIds.length} contract(s)`);
  }

  // Get current ledger and compute start ledger for this window
  let currentLedger: number;
  try {
    currentLedger = await getLatestLedger();
  } catch (err) {
    log(`WARN: Could not fetch latest ledger: ${err instanceof Error ? err.message : err}`);
    return;
  }

  // Approximate ledgers per poll interval (Stellar ~5s per ledger)
  const ledgersPerInterval = Math.ceil(POLL_INTERVAL_MS / 5000) + 10;
  const startLedger = lastLedger > 0 ? lastLedger + 1 : Math.max(1, currentLedger - ledgersPerInterval);
  lastLedger = currentLedger;

  log(`[POLL] Fetching events from ledger ${startLedger} to ${currentLedger}`);

  let events: SorobanEvent[] = [];
  if (contractIds.length > 0) {
    try {
      events = await fetchRecentEvents(contractIds, startLedger);
      log(`[POLL] Fetched ${events.length} event(s)`);
    } catch (err) {
      log(`WARN: Event fetch failed: ${err instanceof Error ? err.message : err}`);
      await sendSlackAlert(
        `⚠️ *Monitor Poll Error*\n` +
        `Failed to fetch events from RPC: ${err instanceof Error ? err.message : err}\n` +
        `Action: Check RPC connectivity and monitor script logs.`
      );
    }
  }

  // Run all checks (independent — one failure does not block others)
  await checkOracleHeartbeat(events).catch((e) => log(`WARN: oracle check error: ${e}`));
  await checkErrorRate(events).catch((e) => log(`WARN: error rate check error: ${e}`));
  await checkFeeSpikes(events).catch((e) => log(`WARN: fee spike check error: ${e}`));
  await checkPauseEvents(events).catch((e) => log(`WARN: pause check error: ${e}`));

  log(`[POLL] Cycle complete`);
}

// ── Main loop ─────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  log(`StellarSwipe Monitor starting`);
  log(`  Network:        ${NETWORK}`);
  log(`  RPC URL:        ${RPC_URL}`);
  log(`  Deploy state:   ${DEPLOY_STATE}`);
  log(`  Poll interval:  ${POLL_INTERVAL_MS / 1000}s`);
  log(`  Oracle threshold: ${ORACLE_HEARTBEAT_THRESHOLD_MS / 60000} min`);
  log(`  Error rate threshold: ${ERROR_RATE_THRESHOLD * 100}%`);
  log(`  Fee spike threshold: ${FEE_SPIKE_THRESHOLD_BPS} bps`);
  log(`  Slack webhook:  ${SLACK_WEBHOOK_URL ? "configured" : "NOT configured (console only)"}`);

  if (!process.env.STELLAR_SOURCE_ACCOUNT && !process.env.STELLAR_ACCOUNT) {
    log("WARN: STELLAR_SOURCE_ACCOUNT not set — some checks may be limited");
  }

  // Run immediately, then on interval
  await poll().catch((e) => log(`ERROR in poll: ${e}`));

  setInterval(() => {
    poll().catch((e) => log(`ERROR in poll: ${e}`));
  }, POLL_INTERVAL_MS);
}

main().catch((err) => {
  console.error("[fatal]", err instanceof Error ? err.message : err);
  process.exit(1);
});
