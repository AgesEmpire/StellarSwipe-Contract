/**
 * test_event_parsing.ts — Verifies Stellar Soroban contract events are parseable
 * with @stellar/stellar-sdk (Soroban RPC / Horizon–compatible ScVal).
 *
 * Convention (this repo): first topic = event name (ScVal::Symbol); full payload
 * is in `value` as standard ScVals (tuples, maps, contract types) so indexers can
 * decode without contract source.
 *
 * Usage (testnet):
 *   export CONTRACT_IDS="C...,C..."   # Soroban contract IDs
 *   export SOROBAN_RPC_URL="https://soroban-testnet.stellar.org"   # optional
 *   npm run test:events
 *
 * Optional: add to config/testnet.json:
 *   "soroban_contracts": { "signal_registry": "C...", "oracle": "C..." }
 */
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import { rpc, scValToNative, StrKey, xdr } from "@stellar/stellar-sdk";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const DEFAULT_RPC = "https://soroban-testnet.stellar.org";

type ParsedEventRow = {
  id: string;
  ledger: number;
  txHash: string;
  contractId: string;
  eventName: string;
  topicsNative: unknown[];
  dataNative: unknown;
};

function loadContractIdsFromConfig(): string[] {
  const configPath = path.join(__dirname, "../config/testnet.json");
  if (!fs.existsSync(configPath)) return [];
  const j = JSON.parse(fs.readFileSync(configPath, "utf8")) as Record<string, unknown>;
  const out: string[] = [];
  const block = j["soroban_contracts"];
  if (block && typeof block === "object" && block !== null) {
    for (const v of Object.values(block)) {
      if (typeof v === "string" && v.startsWith("C") && v.length > 10) {
        out.push(v);
      }
    }
  }
  return out;
}

function contractIdToString(c: unknown): string {
  if (c == null) return "";
  if (typeof c === "string") {
    return c;
  }
  if (c instanceof Buffer) {
    return StrKey.encodeContract(c);
  }
  if (typeof c === "object" && c !== null && "toString" in c) {
    const s = (c as { toString(): string }).toString();
    if (s.startsWith("C") && s.length > 10) {
      return s;
    }
  }
  return String(c);
}

function firstTopicEventName(topics: xdr.ScVal[]): string {
  if (topics.length === 0) return "";
  const sym = scValToNative(topics[0]!);
  return typeof sym === "string" ? sym : String(sym);
}

async function main(): Promise<void> {
  const fromEnv = (process.env.CONTRACT_IDS || "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.startsWith("C"));
  const fromFile = loadContractIdsFromConfig();
  const contractIds = [...new Set([...fromEnv, ...fromFile])];

  if (contractIds.length === 0) {
    console.error(
      'No contract IDs. Set CONTRACT_IDS (C...) or add "soroban_contracts" in config/testnet.json.',
    );
    process.exit(1);
  }

  const rpcUrl = (process.env.SOROBAN_RPC_URL || DEFAULT_RPC).replace(/\/$/, "");
  const server = new rpc.Server(rpcUrl, { allowHttp: rpcUrl.startsWith("http://") });

  const latest = await server.getLatestLedger();
  const endLedger = latest.sequence;
  const lookback = Number.parseInt(process.env.EVENT_LEDGER_LOOKBACK || "20000", 10);
  const startLedger = Math.max(1, endLedger - lookback);
  const limit = Math.min(200, Math.max(1, Number.parseInt(process.env.EVENT_LIMIT || "100", 10)));

  const { events, latestLedger, cursor } = await server.getEvents({
    startLedger,
    endLedger,
    filters: [{ type: "contract", contractIds }],
    limit,
  });

  const parsed: ParsedEventRow[] = [];
  for (const ev of events) {
    if (ev.type !== "contract" || !ev.inSuccessfulContractCall) {
      continue;
    }
    const topicsNative = ev.topic.map((t) => scValToNative(t));
    const dataNative = scValToNative(ev.value);
    parsed.push({
      id: ev.id,
      ledger: ev.ledger,
      txHash: ev.txHash,
      contractId: contractIdToString(ev.contractId as unknown),
      eventName: firstTopicEventName(ev.topic),
      topicsNative,
      dataNative,
    });
  }

  const out = { rpcUrl, startLedger, latestLedger, cursor, count: parsed.length, events: parsed };
  // eslint-disable-next-line no-console
  console.log(JSON.stringify(out, null, 2));
  if (Number(process.env.SAMPLE_OUT || 0) === 1) {
    const samplePath = path.join(__dirname, "sample_parsed_events_testnet.json");
    fs.writeFileSync(samplePath, JSON.stringify(out, null, 2) + "\n", "utf8");
    // eslint-disable-next-line no-console
    console.error(`Wrote ${samplePath}`);
  }
}

void main().catch((e) => {
  // eslint-disable-next-line no-console
  console.error(e);
  process.exit(1);
});
