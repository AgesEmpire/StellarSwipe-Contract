import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { rpc, scValToNative } from "@stellar/stellar-sdk";

type EventSchema = {
  schema_version: string;
  contract: string;
  event_name: string;
  topics_format: string[];
  body_fields: Array<{ name: string; type: string }>;
};

type RootSchema = {
  schema_version: string;
  events: EventSchema[];
};

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, "..");
const SCHEMA_PATH = path.join(ROOT, "docs", "event_schema.json");
const DEFAULT_RPC = "https://soroban-testnet.stellar.org";

function assertSchemaShape(schema: RootSchema): void {
  if (!schema.schema_version) throw new Error("schema_version is required");
  if (!Array.isArray(schema.events) || schema.events.length === 0) {
    throw new Error("events[] must contain at least one event");
  }

  const dedupe = new Set<string>();
  for (const ev of schema.events) {
    if (!ev.contract || !ev.event_name) {
      throw new Error("Each event must include contract + event_name");
    }
    if (!Array.isArray(ev.topics_format)) throw new Error(`topics_format must be array: ${ev.contract}:${ev.event_name}`);
    if (!Array.isArray(ev.body_fields)) throw new Error(`body_fields must be array: ${ev.contract}:${ev.event_name}`);
    if (ev.schema_version !== schema.schema_version) {
      throw new Error(
        `schema_version mismatch for ${ev.contract}:${ev.event_name} -> ${ev.schema_version} != ${schema.schema_version}`,
      );
    }
    const key = `${ev.contract}:${ev.event_name}`;
    if (dedupe.has(key)) throw new Error(`duplicate schema event entry: ${key}`);
    dedupe.add(key);
  }
}

function loadSchema(): RootSchema {
  const raw = fs.readFileSync(SCHEMA_PATH, "utf8");
  return JSON.parse(raw) as RootSchema;
}

function loadContractIdMap(): Record<string, string> {
  const map: Record<string, string> = {};

  const fromEnvMap = process.env.CONTRACT_ID_MAP;
  if (fromEnvMap) {
    Object.assign(map, JSON.parse(fromEnvMap) as Record<string, string>);
  }

  const deployState = process.env.DEPLOY_STATE || path.join(ROOT, "deployments", "testnet.json");
  if (fs.existsSync(deployState)) {
    const state = JSON.parse(fs.readFileSync(deployState, "utf8")) as {
      contracts?: Record<string, { contract_id?: string }>;
    };
    for (const [name, meta] of Object.entries(state.contracts || {})) {
      if (meta?.contract_id) map[name] = meta.contract_id;
    }
  }

  const fromIds = (process.env.CONTRACT_IDS || "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.startsWith("C"));
  for (const id of fromIds) map[`unknown_${id.slice(0, 8)}`] = id;

  return map;
}

function normalizeEventName(topic0: unknown, topic1: unknown): string {
  const p0 = typeof topic0 === "string" ? topic0 : String(topic0 ?? "");
  const p1 = typeof topic1 === "string" ? topic1 : String(topic1 ?? "");
  if (!p0) return "";
  if (p0 === "gov" || p0 === "qv" || p0 === "upgrade") return `${p0}:${p1}`;
  return p0;
}

async function validateAgainstTestnet(schema: RootSchema, contractMap: Record<string, string>): Promise<void> {
  const entries = Object.entries(contractMap).filter(([, id]) => id && id.startsWith("C"));
  if (entries.length === 0) {
    console.log("No contract IDs found. Schema structure validated; live testnet validation skipped.");
    console.log("Provide CONTRACT_ID_MAP, DEPLOY_STATE, or CONTRACT_IDS to validate against RPC events.");
    return;
  }

  const byId = new Map(entries.map(([name, id]) => [id, name]));
  const ids = [...byId.keys()];
  const server = new rpc.Server(process.env.SOROBAN_RPC_URL || DEFAULT_RPC, { allowHttp: false });

  const latest = await server.getLatestLedger();
  const lookback = Number.parseInt(process.env.EVENT_LEDGER_LOOKBACK || "20000", 10);
  const startLedger = Math.max(1, latest.sequence - lookback);
  const limit = Math.min(200, Math.max(1, Number.parseInt(process.env.EVENT_LIMIT || "200", 10)));

  const res = await server.getEvents({
    startLedger,
    endLedger: latest.sequence,
    limit,
    filters: [{ type: "contract", contractIds: ids }],
  });

  const schemaSet = new Set(schema.events.map((e) => `${e.contract}:${e.event_name}`));
  const seen = new Set<string>();
  const missing: string[] = [];

  for (const ev of res.events) {
    if (ev.type !== "contract") continue;
    const contractId = String(ev.contractId ?? "");
    const contract = byId.get(contractId) || "unknown";
    const t0 = scValToNative(ev.topic[0]);
    const t1 = ev.topic[1] ? scValToNative(ev.topic[1]) : "";
    const name = normalizeEventName(t0, t1);
    if (!name) continue;
    const key = `${contract}:${name}`;
    seen.add(key);
    if (!schemaSet.has(key)) missing.push(key);
  }

  console.log(
    JSON.stringify(
      {
        rpc: process.env.SOROBAN_RPC_URL || DEFAULT_RPC,
        ledgers: { start: startLedger, end: latest.sequence },
        event_count: res.events.length,
        unique_seen_events: [...seen].sort(),
        missing_schema_entries: [...new Set(missing)].sort(),
      },
      null,
      2,
    ),
  );

  if (missing.length > 0) {
    throw new Error(`Missing schema entries for ${new Set(missing).size} observed event(s).`);
  }
}

async function main(): Promise<void> {
  const schema = loadSchema();
  assertSchemaShape(schema);
  const contractMap = loadContractIdMap();
  await validateAgainstTestnet(schema, contractMap);
}

void main().catch((err) => {
  console.error(err);
  process.exit(1);
});
