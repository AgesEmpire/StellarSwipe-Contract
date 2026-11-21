/**
 * Deployment registry reader (Issue #881).
 *
 * `deployments/registry.json` is the single source of truth for "which contract
 * is deployed where". Scripts and operators resolve addresses through this
 * module instead of reaching into per-deploy state files.
 *
 * CLI:
 *   npx tsx deployment_registry.ts list testnet
 *   npx tsx deployment_registry.ts get testnet stake_vault
 */

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REGISTRY_PATH = resolve(REPO_ROOT, "deployments/registry.json");

export interface ContractEntry {
  address: string | null;
  version: number;
}

export interface NetworkEntry {
  network_passphrase: string;
  rpc_url: string;
  manifest: string;
  deployed_at: string | null;
  contracts: Record<string, ContractEntry>;
}

export interface Registry {
  schema_version: number;
  networks: Record<string, NetworkEntry>;
}

export function loadRegistry(path: string = REGISTRY_PATH): Registry {
  const registry = JSON.parse(readFileSync(path, "utf8")) as Registry;
  if (registry.schema_version !== 1) {
    throw new Error(
      `Unsupported registry schema_version ${registry.schema_version} (expected 1)`,
    );
  }
  return registry;
}

export function getNetwork(registry: Registry, network: string): NetworkEntry {
  const entry = registry.networks[network];
  if (!entry) {
    const known = Object.keys(registry.networks).join(", ");
    throw new Error(`Unknown network "${network}". Known networks: ${known}`);
  }
  return entry;
}

/** Resolves a deployed contract address, throwing if it has not been deployed yet. */
export function getContractId(
  registry: Registry,
  network: string,
  contract: string,
): string {
  const entry = getNetwork(registry, network).contracts[contract];
  if (!entry) {
    throw new Error(`Contract "${contract}" is not in the ${network} registry`);
  }
  if (!entry.address) {
    throw new Error(
      `Contract "${contract}" has no address on ${network} — deploy it first, then run \`record\``,
    );
  }
  return entry.address;
}

/** Records a freshly deployed address back into the registry. */
export function recordDeployment(
  network: string,
  contract: string,
  address: string,
  path: string = REGISTRY_PATH,
): void {
  const registry = loadRegistry(path);
  const networkEntry = getNetwork(registry, network);
  const contractEntry = networkEntry.contracts[contract];
  if (!contractEntry) {
    throw new Error(`Contract "${contract}" is not in the ${network} registry`);
  }
  contractEntry.address = address;
  networkEntry.deployed_at = new Date().toISOString();
  writeFileSync(path, `${JSON.stringify(registry, null, 2)}\n`);
}

function main(): void {
  const [command, network, contract] = process.argv.slice(2);
  const registry = loadRegistry();

  switch (command) {
    case "list": {
      const entry = getNetwork(registry, network);
      console.log(`${network} (${entry.rpc_url})`);
      for (const [name, { address, version }] of Object.entries(entry.contracts)) {
        console.log(`  ${name.padEnd(16)} v${version}  ${address ?? "<not deployed>"}`);
      }
      break;
    }
    case "get":
      console.log(getContractId(registry, network, contract));
      break;
    default:
      console.error("usage: deployment_registry.ts <list|get> <network> [contract]");
      process.exit(1);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
