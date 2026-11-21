#!/usr/bin/env tsx
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import {
  BASE_FEE,
  Contract,
  Keypair,
  Networks,
  TransactionBuilder,
  nativeToScVal,
  rpc,
} from "@stellar/stellar-sdk";

type NetworkName = "testnet" | "mainnet";

interface DeploymentContractRef {
  contract_id?: string;
  address?: string;
  id?: string;
}

interface DeploymentManifest {
  network?: string;
  rpc_url?: string;
  network_passphrase?: string;
  contracts?: Record<string, string | DeploymentContractRef>;
  assets?: Record<string, string | DeploymentContractRef>;
}

interface TradeAsset {
  code: string;
  issuer: string | null;
}

function readDeploymentContract(
  manifest: DeploymentManifest,
  name: string,
  kind: "contract" | "asset" = "contract"
): string {
  const table = kind === "contract" ? manifest.contracts : manifest.assets;
  const value = table?.[name];

  if (typeof value === "string" && value.length > 0) {
    return value;
  }

  if (value && typeof value === "object") {
    const ref = value.contract_id ?? value.address ?? value.id;
    if (typeof ref === "string" && ref.length > 0) {
      return ref;
    }
  }

  throw new Error(
    `Missing ${kind} "${name}" in deployments/testnet.json. ` +
      `Populate the manifest with a live testnet address before running e2e.`
  );
}

function loadSigner(envVar: string): Keypair {
  const secret = process.env[envVar];
  if (!secret) {
    throw new Error(`Missing required env var ${envVar}`);
  }
  return Keypair.fromSecret(secret);
}

async function loadManifest(): Promise<DeploymentManifest> {
  const deploymentPath = process.env.STELLAR_E2E_DEPLOYMENT_STATE ?? "deployments/testnet.json";
  const raw = await readFile(resolve(process.cwd(), deploymentPath), "utf8");
  return JSON.parse(raw) as DeploymentManifest;
}

async function wait(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function invokeContract(
  server: rpc.Server,
  networkPassphrase: string,
  contractId: string,
  signer: Keypair,
  method: string,
  args: unknown[]
): Promise<unknown> {
  const account = await server.getAccount(signer.publicKey());
  const contract = new Contract(contractId);
  const tx = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase,
  })
    .addOperation(contract.call(method, ...args.map((arg) => nativeToScVal(arg))))
    .setTimeout(180)
    .build();

  const simulated = await server.simulateTransaction(tx);
  if (rpc.Api.isSimulationError(simulated)) {
    throw new Error(`Simulation failed for ${method}: ${simulated.error}`);
  }

  const prepared = rpc.assembleTransaction(tx, simulated).build();
  prepared.sign(signer);

  const sent = await server.sendTransaction(prepared);
  if (sent.status === "PENDING") {
    const result = await server.pollTransaction(sent.hash);
    assert.equal(result.status, "SUCCESS", `Transaction ${method} failed: ${result.status}`);
    return result;
  }

  assert.equal(sent.status, "SUCCESS", `Transaction ${method} failed: ${sent.status}`);
  return sent;
}

async function runGovernanceFlow(
  server: rpc.Server,
  networkPassphrase: string,
  manifest: DeploymentManifest,
  admin: Keypair
): Promise<void> {
  const governanceId = readDeploymentContract(manifest, "governance");
  const totalSupply = process.env.STELLAR_E2E_GOVERNANCE_SUPPLY ?? "1000000000000000";
  const votingPower = BigInt(totalSupply) / 5n;

  await invokeContract(server, networkPassphrase, governanceId, admin, "stake", [
    admin.publicKey(),
    votingPower,
  ]);

  await invokeContract(server, networkPassphrase, governanceId, admin, "configure_governance", [
      admin.publicKey(),
      {
        min_proposal_threshold: 1n,
        voting_period: 1,
        voting_delay: 0,
        quorum_threshold: 1000,
        approval_threshold: 5000,
        execution_delay: 0,
      },
  ]);

  const proposalType = {
    FeatureToggle: ["nightly_e2e", true],
  };

  await invokeContract(server, networkPassphrase, governanceId, admin, "create_proposal", [
    admin.publicKey(),
    proposalType,
    "Nightly e2e feature toggle",
    "Exercise governance proposal creation in testnet e2e",
    Buffer.from("nightly-e2e"),
  ]);

  const proposalId = 1;
  await invokeContract(server, networkPassphrase, governanceId, admin, "cast_vote", [
    proposalId,
    admin.publicKey(),
    { For: null },
  ]);

  await wait(2_500);

  const finalizeResult = await invokeContract(
    server,
    networkPassphrase,
    governanceId,
    admin,
    "finalize_proposal",
    [proposalId]
  );

  console.log("[governance]", JSON.stringify({ proposalId, finalizeResult }, null, 2));
}

async function runProviderStakingFlow(
  server: rpc.Server,
  networkPassphrase: string,
  manifest: DeploymentManifest,
  provider: Keypair
): Promise<void> {
  const signalRegistryId = readDeploymentContract(manifest, "signal_registry");

  await invokeContract(server, networkPassphrase, signalRegistryId, provider, "stake_tokens", [
    provider.publicKey(),
    100000000n,
  ]);

  const verification = await invokeContract(
    server,
    networkPassphrase,
    signalRegistryId,
    provider,
    "check_verification_eligibility",
    [provider.publicKey()]
  );

  console.log("[staking]", JSON.stringify({ verification }, null, 2));
}

async function runTradeFlow(
  server: rpc.Server,
  networkPassphrase: string,
  manifest: DeploymentManifest,
  trader: Keypair
): Promise<void> {
  const portfolioId = readDeploymentContract(manifest, "user_portfolio");

  await invokeContract(server, networkPassphrase, portfolioId, trader, "open_position", [
    trader.publicKey(),
    100n,
    1000n,
  ]);

  const positionId = 1;
  await invokeContract(server, networkPassphrase, portfolioId, trader, "close_position", [
    trader.publicKey(),
    positionId,
    200n,
    120n,
    0,
    trader.publicKey(),
    1n,
  ]);

  const pnl = await invokeContract(server, networkPassphrase, portfolioId, trader, "get_pnl", [
    trader.publicKey(),
  ]);

  console.log("[trade]", JSON.stringify({ positionId, pnl }, null, 2));
}

async function runFeeCollectionFlow(
  server: rpc.Server,
  networkPassphrase: string,
  manifest: DeploymentManifest,
  admin: Keypair,
  trader: Keypair
): Promise<void> {
  const feeCollectorId = readDeploymentContract(manifest, "fee_collector");
  const oracleId = readDeploymentContract(manifest, "oracle");
  const feeToken = process.env.STELLAR_E2E_FEE_TOKEN ?? readDeploymentContract(manifest, "fee_token", "asset");
  const tradeAsset: TradeAsset = {
    code: process.env.STELLAR_E2E_TRADE_ASSET_CODE ?? "XLM",
    issuer: process.env.STELLAR_E2E_TRADE_ASSET_ISSUER ?? null,
  };

  await invokeContract(server, networkPassphrase, feeCollectorId, admin, "set_oracle_contract", [
    oracleId,
  ]);

  await invokeContract(server, networkPassphrase, feeCollectorId, admin, "set_fee_rate", [30]);
  await invokeContract(server, networkPassphrase, feeCollectorId, admin, "set_burn_rate", [1000]);

  const waiveResult = await invokeContract(
    server,
    networkPassphrase,
    feeCollectorId,
    trader,
    "collect_fee",
    [trader.publicKey(), feeToken, 1000000n, tradeAsset]
  );

  const chargeResult = await invokeContract(
    server,
    networkPassphrase,
    feeCollectorId,
    trader,
    "collect_fee",
    [trader.publicKey(), feeToken, 1000000n, tradeAsset]
  );

  const claimResult = await invokeContract(
    server,
    networkPassphrase,
    feeCollectorId,
    trader,
    "claim_fees",
    [trader.publicKey(), feeToken]
  );

  console.log(
    "[fees]",
    JSON.stringify({ waiveResult, chargeResult, claimResult }, null, 2)
  );
}

async function main(): Promise<void> {
  const manifest = await loadManifest();
  const selectedNetwork: NetworkName = (manifest.network ?? process.env.STELLAR_NETWORK ?? "testnet")
    .toLowerCase() === "mainnet"
    ? "mainnet"
    : "testnet";
  assert.equal(selectedNetwork, "testnet", "e2e test is intended to run against testnet");

  const rpcUrl = process.env.STELLAR_E2E_RPC_URL ?? manifest.rpc_url ?? "https://soroban-testnet.stellar.org";
  const networkPassphrase =
    process.env.STELLAR_E2E_NETWORK_PASSPHRASE ??
    manifest.network_passphrase ??
    Networks.TESTNET;
  const server = new rpc.Server(rpcUrl, { allowHttp: rpcUrl.startsWith("http://") });

  const admin = loadSigner("STELLAR_E2E_ADMIN_SECRET");
  const trader = loadSigner("STELLAR_E2E_TRADER_SECRET");
  const provider = loadSigner("STELLAR_E2E_PROVIDER_SECRET");

  await runGovernanceFlow(server, networkPassphrase, manifest, admin);
  await runProviderStakingFlow(server, networkPassphrase, manifest, provider);
  await runTradeFlow(server, networkPassphrase, manifest, trader);
  await runFeeCollectionFlow(server, networkPassphrase, manifest, admin, trader);

  console.log(
    JSON.stringify(
      {
        network: selectedNetwork,
        rpcUrl,
        status: "PASS",
      },
      null,
      2
    )
  );
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.stack ?? error.message : String(error));
    process.exit(1);
  });
}
