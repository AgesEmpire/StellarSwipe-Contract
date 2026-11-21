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
import * as crypto from "crypto";

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

// ── Manifest / WASM hash validation (Issue #1454) ──────────────────────────
//
// Deployment manifests (deployments/<network>.manifest.json) record a
// "checksum" per artifact (sha256 of the built wasm) and a "wasmPath". Prior
// to this change, deploy.ts never read the manifest at all, so a stale or
// tampered manifest — or a wasm rebuilt from different source — could be
// deployed without anyone noticing a mismatch. We now compute the actual
// sha256 of the built wasm and fail loudly if it disagrees with the
// manifest-recorded checksum, or if the manifest is missing/incomplete.

interface ManifestArtifact {
  contractName: string;
  wasmPath: string;
  checksum: string;
  sourceRevision?: string;
}

interface Manifest {
  network: string;
  artifacts: ManifestArtifact[];
}

function sha256File(filePath: string): string {
  const buf = fs.readFileSync(filePath);
  return `sha256:${crypto.createHash("sha256").update(buf).digest("hex")}`;
}

function loadManifest(network: string): Manifest {
  const manifestPath = path.resolve(__dirname, `../deployments/${network}.manifest.json`);
  if (!fs.existsSync(manifestPath)) {
    throw new Error(`ERROR: No deployment manifest found for network "${network}" at ${manifestPath}`);
  }
  return JSON.parse(fs.readFileSync(manifestPath, "utf8"));
}

/**
 * Verify that the manifest-recorded checksum for `contractName` matches the
 * sha256 of the actual wasm file on disk at `wasmAbsPath`. Throws (failing
 * loudly) on any mismatch or missing entry — this must run before any
 * `stellar contract deploy` invocation so a bad manifest never reaches chain.
 */
function verifyWasmAgainstManifest(
  network: string,
  contractName: string,
  wasmAbsPath: string
): void {
  const manifest = loadManifest(network);
  const entry = manifest.artifacts.find((a) => a.contractName === contractName);
  if (!entry) {
    throw new Error(
      `ERROR: manifest for "${network}" has no entry for contract "${contractName}"; refusing to deploy.`
    );
  }
  if (!fs.existsSync(wasmAbsPath)) {
    throw new Error(`ERROR: expected built wasm not found at ${wasmAbsPath}`);
  }
  const actualChecksum = sha256File(wasmAbsPath);
  if (entry.checksum.startsWith("sha256:0000000000")) {
    throw new Error(
      `ERROR: manifest checksum for "${contractName}" is a placeholder (never populated); refusing to deploy.`
    );
  }
  if (actualChecksum !== entry.checksum) {
    throw new Error(
      `ERROR: WASM hash mismatch for "${contractName}".\n` +
      `  manifest checksum: ${entry.checksum}\n` +
      `  actual wasm hash:  ${actualChecksum}\n` +
      `  wasm path:         ${wasmAbsPath}\n` +
      `Refusing to deploy a contract whose built artifact does not match the recorded manifest hash.`
    );
  }
  console.log(`Manifest checksum verified for "${contractName}": ${actualChecksum}`);
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

const signalRegistryWasm = `${wasmDir}/signal_registry.wasm`;
verifyWasmAgainstManifest(NETWORK, "signal_registry", signalRegistryWasm);

const signalRegistryId = deployContract(signalRegistryWasm);
console.log(`signal_registry deployed: ${signalRegistryId}`);

stellar(
  `contract invoke --id ${signalRegistryId} --source-account ${SOURCE} --network ${NETWORK} ` +
  `-- initialize --admin ${config.admin}`
);

console.log("Done.");
