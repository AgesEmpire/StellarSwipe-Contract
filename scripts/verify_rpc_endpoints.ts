#!/usr/bin/env tsx
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

type NetworkName = "testnet" | "mainnet";

interface RpcEndpointConfig {
  primary_rpc: string;
  fallback_rpc: string;
  horizon_url: string;
  network_passphrase: string;
}

type RpcEndpointMap = Record<NetworkName, RpcEndpointConfig>;

async function rpcGetNetwork(rpcUrl: string): Promise<{ passphrase?: string }> {
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "getNetwork",
    }),
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status}: ${response.statusText}`);
  }

  const json = (await response.json()) as {
    error?: { message?: string };
    result?: { passphrase?: string };
  };

  if (json.error) {
    throw new Error(json.error.message ?? "RPC returned an error");
  }

  return json.result ?? {};
}

async function main() {
  const networkName = (process.env.NEXT_PUBLIC_STELLAR_NETWORK ?? "testnet").toLowerCase();
  const selectedNetwork: NetworkName = networkName === "mainnet" ? "mainnet" : "testnet";
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const configPath = resolve(repoRoot, "config/rpc_endpoints.json");
  const raw = await readFile(configPath, "utf8");
  const config = JSON.parse(raw) as RpcEndpointMap;
  const selected = config[selectedNetwork];

  const endpoints = [selected.primary_rpc, selected.fallback_rpc].filter(
    (value, index, list) => list.indexOf(value) === index
  );

  const results: Array<{ url: string; passphrase: string }> = [];

  for (const url of endpoints) {
    const network = await rpcGetNetwork(url);
    if (!network.passphrase) {
      throw new Error(`RPC ${url} did not return a network passphrase`);
    }
    if (network.passphrase !== selected.network_passphrase) {
      throw new Error(
        `RPC ${url} returned passphrase "${network.passphrase}" but expected "${selected.network_passphrase}"`
      );
    }
    results.push({ url, passphrase: network.passphrase });
  }

  console.log(JSON.stringify({ network: selectedNetwork, results }, null, 2));
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
