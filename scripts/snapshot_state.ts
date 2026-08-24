#!/usr/bin/env tsx
import fs from "fs";
import path from "path";
import { rpc, Address, Contract, xdr, scValToNative } from "@stellar/stellar-sdk";

const rpcUrl =
  process.argv[3] ||
  process.env.SOROBAN_RPC_URL ||
  "https://horizon-testnet.stellar.org";
const contractId = process.argv[2];

function usage(): never {
  console.error("Usage: npx tsx scripts/snapshot_state.ts <contract-id> [rpc-url]");
  process.exit(1);
}

function sanitizeFileName(value: string): string {
  return value.replace(/[^a-zA-Z0-9._-]/g, "_");
}

function safeJson(value: unknown): unknown {
  if (typeof value === "bigint") {
    return value.toString();
  }
  if (value instanceof Uint8Array) {
    return Buffer.from(value).toString("hex");
  }
  if (Array.isArray(value)) {
    return value.map(safeJson);
  }
  if (value && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, val] of Object.entries(value)) {
      result[key] = safeJson(val);
    }
    return result;
  }
  return value;
}

function decodeLedgerKey(key: xdr.LedgerKey): unknown {
  const switchName = key.switch().name;
  if (switchName === "contractData") {
    const contractKey = key.contractData();
    return {
      type: "contract_data",
      contract: Address.fromScAddress(contractKey.contract()).toString(),
      durability: contractKey.durability().switch().name,
      key: safeJson(scValToNative(contractKey.key())),
    };
  }
  if (switchName === "contractInstance") {
    const contractInstance = key.contractInstance();
    return {
      type: "contract_instance",
      contract: Address.fromScAddress(contractInstance.contract()).toString(),
    };
  }
  return {
    type: switchName,
    raw: key.toXDR("base64"),
  };
}

function decodeLedgerValue(val: xdr.LedgerEntryData): unknown {
  const switchName = val.switch().name;
  if (switchName === "contractData") {
    return {
      type: "contract_data",
      value: safeJson(scValToNative(val.contractData().val())),
    };
  }
  if (switchName === "contractCode") {
    const codeEntry = val.contractCode();
    return {
      type: "contract_code",
      hash: Buffer.from(codeEntry.hash()).toString("hex"),
      xdr: codeEntry.toXDR("base64"),
    };
  }
  return {
      type: switchName,
      raw: val.toXDR("base64"),
  };
}

async function snapshotState(contractId: string, rpcUrl: string): Promise<string> {
  const server = new rpc.Server(rpcUrl);
  const contract = new Contract(contractId);
  const response = await server.getLedgerEntries(contract.getFootprint());

  const decoded = {
    latestLedger: response.latestLedger,
    entries: response.entries.map((entry) => ({
      lastModifiedLedgerSeq: entry.lastModifiedLedgerSeq,
      liveUntilLedgerSeq: entry.liveUntilLedgerSeq,
      key: decodeLedgerKey(entry.key),
      value: decodeLedgerValue(entry.val),
      rawKeyXdr: entry.key.toXDR("base64"),
      rawValueXdr: entry.val.toXDR("base64"),
    })),
  };

  const outputDir = path.join(process.cwd(), "snapshots");
  await fs.promises.mkdir(outputDir, { recursive: true });
  const outputPath = path.join(
    outputDir,
    `${sanitizeFileName(contractId)}_${response.latestLedger}.json`
  );
  await fs.promises.writeFile(outputPath, JSON.stringify(decoded, null, 2), "utf8");
  return outputPath;
}

async function main(): Promise<void> {
  if (!contractId) {
    usage();
  }

  console.log(`Snapshotting contract state for ${contractId} using ${rpcUrl}`);
  try {
    const outputPath = await snapshotState(contractId, rpcUrl);
    console.log(`Saved state snapshot to ${outputPath}`);
  } catch (err) {
    console.error("Failed to snapshot state:", err instanceof Error ? err.message : err);
    process.exit(1);
  }
}

if (process.argv[1]?.endsWith("snapshot_state.ts") || process.argv[1]?.endsWith("snapshot_state.js")) {
  main();
}
