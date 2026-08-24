#!/usr/bin/env ts-node
/**
 * deploy.ts — Network-aware deploy script for StellarSwipe contracts.
 *
 * Usage:
 *   STELLAR_NETWORK=testnet STELLAR_SOURCE_ACCOUNT=<key> STELLAR_ADMIN_ADDRESS=<G...> npx ts-node scripts/deploy.ts
 *   STELLAR_NETWORK=mainnet STELLAR_SOURCE_ACCOUNT=<key> STELLAR_ADMIN_ADDRESS=<G...> npx ts-node scripts/deploy.ts
 *
 * Reads config from config/{network}.json. Mainnet is blocked in automated tests.
 */

import { execSync } from "child_process";
import * as fs from "fs";
import * as path from "path";

// ── Types ────────────────────────────────────────────────────────────────────

interface NetworkConfig {
  /** Minimum stake amount in stroops (i128) */
  min_stake: number;
  /** Maximum fee rate in basis points (u32) */
  max_fee_rate: number;
  /** Soroban contract address of the oracle */
  oracle_address: string;
  /** Admin account StrKey (G...) */
  admin: string;
}

// ── Guards ───────────────────────────────────────────────────────────────────

const NETWORK = process.env.STELLAR_NETWORK ?? "testnet";

if (process.env.CI === "true" && NETWORK === "mainnet") {
  console.error("ERROR: mainnet config must never be used in automated tests.");
  process.exit(1);
}

// ── Load config ──────────────────────────────────────────────────────────────

const configPath = path.resolve(__dirname, `../config/${NETWORK}.json`);
if (!fs.existsSync(configPath)) {
  console.error(`ERROR: No config found for network "${NETWORK}" at ${configPath}`);
  process.exit(1);
}

const config: NetworkConfig = JSON.parse(fs.readFileSync(configPath, "utf8"));

// Validate no placeholder values slipped through
for (const [key, val] of Object.entries(config)) {
  if (typeof val === "string" && val.startsWith("REPLACE_WITH_")) {
    console.error(`ERROR: config field "${key}" has not been set (value: ${val})`);
    process.exit(1);
  }
}

// ── Deploy ───────────────────────────────────────────────────────────────────

const SOURCE = process.env.STELLAR_SOURCE_ACCOUNT ?? process.env.STELLAR_ACCOUNT;
if (!SOURCE) {
  console.error("ERROR: set STELLAR_SOURCE_ACCOUNT or STELLAR_ACCOUNT");
  process.exit(1);
}

console.log(`Deploying to ${NETWORK} with config:`, config);

function stellar(args: string): string {
  return execSync(`stellar ${args}`, { encoding: "utf8" }).trim();
}

function deployContract(wasmPath: string): string {
  const out = stellar(
    `contract deploy --wasm ${wasmPath} --source-account ${SOURCE} --network ${NETWORK}`
  );
  const match = out.match(/C[2-7A-Z]{55}/);
  if (!match) throw new Error(`Could not parse contract ID from: ${out}`);
  return match[0];
}

// Deploy signal_registry
const wasmDir = path.resolve(
  __dirname,
  "../stellar-swipe/target/wasm32-unknown-unknown/release"
);

const signalRegistryId = deployContract(`${wasmDir}/signal_registry.wasm`);
console.log(`signal_registry deployed: ${signalRegistryId}`);

stellar(
  `contract invoke --id ${signalRegistryId} --source-account ${SOURCE} --network ${NETWORK} ` +
  `-- initialize --admin ${config.admin}`
);

console.log("Done.");
