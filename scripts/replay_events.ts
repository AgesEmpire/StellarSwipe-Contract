#!/usr/bin/env tsx
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import { rpc, scValToNative, StrKey, xdr } from "@stellar/stellar-sdk";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_RPC = "https://soroban-testnet.stellar.org";

function usage(): never {
  console.error(`Usage: tsx replay_events.ts --contract C... --start <ledger> --end <ledger> [--rpc <url>] [--event <name>] [--limit <n>] [--output <file>]`);
  process.exit(1);
}

type Args = {
  contractId?: string;
  rpcUrl: string;
  startLedger?: number;
  endLedger?: number;
  eventName?: string;
  limit: number;
  outputPath?: string;
};

function parseArgs(argv: string[]): Args {
  const args: Args = { rpcUrl: DEFAULT_RPC, limit: 200 };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    switch (arg) {
      case "--contract":
        args.contractId = argv[++i];
        break;
      case "--rpc":
        args.rpcUrl = argv[++i];
        break;
      case "--start":
        args.startLedger = Number(argv[++i]);
        break;
      case "--end":
        args.endLedger = Number(argv[++i]);
        break;
      case "--event":
        args.eventName = argv[++i];
        break;
      case "--limit":
        args.limit = Number(argv[++i]);
        break;
      case "--output":
        args.outputPath = argv[++i];
        break;
      default:
        console.error(`Unknown argument: ${arg}`);
        usage();
    }
  }
  if (!args.contractId || args.startLedger == null || args.endLedger == null) {
    usage();
  }
  return args;
}

function contractIdToString(c: unknown): string {
  if (typeof c === "string") return c;
  if (c instanceof Buffer) return StrKey.encodeContract(c);
  if (typeof c === "object" && c !== null && "toString" in c) {
    const s = (c as { toString(): string }).toString();
    if (s.startsWith("C") && s.length > 10) return s;
  }
  return String(c);
}

function firstTopicEventName(topics: xdr.ScVal[]): string {
  if (topics.length === 0) return "";
  const sym = scValToNative(topics[0]!);
  return typeof sym === "string" ? sym : String(sym);
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const server = new rpc.Server(args.rpcUrl.replace(/\/+$/, ""), {
    allowHttp: args.rpcUrl.startsWith("http://"),
  });

  const outEvents: unknown[] = [];
  let cursor: string | undefined = undefined;
  let page = 0;

  while (true) {
    const filter = [{ type: "contract", contractIds: [args.contractId!] }];
    const response = await server.getEvents({
      startLedger: args.startLedger!,
      endLedger: args.endLedger!,
      filters: filter,
      limit: args.limit,
      cursor,
    });

    const parsed = response.events
      .filter((ev) => ev.type === "contract" && ev.inSuccessfulContractCall)
      .map((ev) => ({
        id: ev.id,
        ledger: ev.ledger,
        txHash: ev.txHash,
        contractId: contractIdToString(ev.contractId as unknown),
        eventName: firstTopicEventName(ev.topic),
        topicsNative: ev.topic.map((t) => scValToNative(t)),
        dataNative: scValToNative(ev.value),
      }));

    outEvents.push(...parsed);
    page += 1;

    if (!response.cursor || response.events.length < args.limit) {
      break;
    }

    cursor = response.cursor;
  }

  const result = {
    rpcUrl: args.rpcUrl,
    contractId: args.contractId,
    startLedger: args.startLedger,
    endLedger: args.endLedger,
    eventName: args.eventName || null,
    count: outEvents.length,
    events: outEvents,
  };

  const output = JSON.stringify(result, null, 2);
  console.log(output);
  if (args.outputPath) {
    fs.writeFileSync(path.resolve(args.outputPath), output + "\n", "utf8");
  }
}

void main().catch((error) => {
  console.error(error);
  process.exit(1);
});
