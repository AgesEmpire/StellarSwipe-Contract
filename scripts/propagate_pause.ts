#!/usr/bin/env ts-node
/**
 * propagate_pause.ts — Atomic cross-contract emergency pause propagation.
 *
 * Issue #1453: each contract (fee_collector, stake_vault, ...) exposes its
 * own admin-gated pause flag (set_collection_paused / set_paused), but there
 * was no single entry point that pauses them together, and no documented
 * behavior for what happens if pausing contract N fails after contracts
 * 1..N-1 already succeeded. That left the system able to end up "half
 * paused": some contracts halted, others still live, with no record of the
 * partial failure and no automatic rollback.
 *
 * This script makes cross-contract pause propagation an explicit,
 * best-effort-atomic operation:
 *   1. Pause each contract in `PAUSABLE_CONTRACTS`, in order.
 *   2. If any pause call fails, immediately roll back by un-pausing every
 *      contract that was *successfully* paused in this run (reverse order),
 *      so a partial failure never leaves the system straddling paused and
 *      unpaused state.
 *   3. If rollback itself fails for some contract, that is reported loudly
 *      (never swallowed) so an operator must intervene manually — this is
 *      the one case true atomicity cannot be guaranteed on-chain without a
 *      shared governance contract, so we fail as loudly as possible instead
 *      of pretending nothing happened.
 *
 * Usage:
 *   STELLAR_NETWORK=testnet STELLAR_SOURCE_ACCOUNT=<key> npx ts-node scripts/propagate_pause.ts --pause
 *   STELLAR_NETWORK=testnet STELLAR_SOURCE_ACCOUNT=<key> npx ts-node scripts/propagate_pause.ts --unpause
 */

import { execSync } from "child_process";
import * as fs from "fs";
import * as path from "path";

const NETWORK = process.env.STELLAR_NETWORK ?? "testnet";
const SOURCE = process.env.STELLAR_SOURCE_ACCOUNT ?? process.env.STELLAR_ACCOUNT;

if (!SOURCE) {
  console.error("ERROR: set STELLAR_SOURCE_ACCOUNT or STELLAR_ACCOUNT");
  process.exit(1);
}

const desiredPaused = process.argv.includes("--pause")
  ? true
  : process.argv.includes("--unpause")
  ? false
  : null;

if (desiredPaused === null) {
  console.error("ERROR: pass --pause or --unpause");
  process.exit(1);
}

interface PausableContract {
  /** Key in deployments/registry.json under networks.<net>.contracts */
  registryKey: string;
  /** The invoke function name used to toggle this contract's pause flag. */
  setPauseFn: string;
}

// Contracts known to expose an admin-gated pause toggle. Keep in sync with
// any new contract that adds pause support.
const PAUSABLE_CONTRACTS: PausableContract[] = [
  { registryKey: "fee_collector", setPauseFn: "set_collection_paused" },
  { registryKey: "stake_vault", setPauseFn: "set_paused" },
];

function loadContractAddress(network: string, registryKey: string): string {
  const registryPath = path.resolve(__dirname, "../deployments/registry.json");
  const registry = JSON.parse(fs.readFileSync(registryPath, "utf8"));
  const entry = registry.networks?.[network]?.contracts?.[registryKey];
  if (!entry || !entry.address) {
    throw new Error(
      `ERROR: no deployed address recorded for "${registryKey}" on network "${network}"`
    );
  }
  return entry.address;
}

function invokeSetPaused(contractId: string, fnName: string, paused: boolean): void {
  // fee_collector's setter takes (admin, paused); stake_vault's takes only
  // (paused) and relies on the invoker being the admin via require_auth.
  const args =
    fnName === "set_collection_paused"
      ? `-- ${fnName} --admin ${SOURCE} --paused ${paused}`
      : `-- ${fnName} --paused ${paused}`;
  execSync(
    `stellar contract invoke --id ${contractId} --source-account ${SOURCE} --network ${NETWORK} ${args}`,
    { encoding: "utf8", stdio: "pipe" }
  );
}

function propagatePause(paused: boolean): void {
  const succeeded: PausableContract[] = [];

  for (const contract of PAUSABLE_CONTRACTS) {
    const contractId = loadContractAddress(NETWORK, contract.registryKey);
    try {
      console.log(`Setting paused=${paused} on ${contract.registryKey} (${contractId})...`);
      invokeSetPaused(contractId, contract.setPauseFn, paused);
      succeeded.push(contract);
      console.log(`  OK: ${contract.registryKey} paused=${paused}`);
    } catch (err) {
      console.error(
        `ERROR: failed to set paused=${paused} on ${contract.registryKey}: ${err}`
      );
      console.error(
        `Rolling back ${succeeded.length} contract(s) already toggled, in reverse order, ` +
        `to avoid leaving the system in a mixed pause state...`
      );
      rollback(succeeded, !paused);
      throw new Error(
        `Pause propagation aborted after failure on "${contract.registryKey}". ` +
        `Successfully-toggled contracts were rolled back to paused=${!paused}.`
      );
    }
  }

  console.log(`Pause propagation complete: all contracts set to paused=${paused}.`);
}

/**
 * Best-effort rollback: restore every contract in `toRollback` to
 * `rollbackPaused`. Any rollback failure is reported individually and does
 * NOT get silently absorbed — operators must be told exactly which
 * contracts are left in the uncertain state.
 */
function rollback(toRollback: PausableContract[], rollbackPaused: boolean): void {
  const failures: string[] = [];
  for (const contract of [...toRollback].reverse()) {
    try {
      const contractId = loadContractAddress(NETWORK, contract.registryKey);
      invokeSetPaused(contractId, contract.setPauseFn, rollbackPaused);
      console.log(`  Rolled back ${contract.registryKey} to paused=${rollbackPaused}`);
    } catch (rollbackErr) {
      failures.push(contract.registryKey);
      console.error(
        `  CRITICAL: rollback FAILED for ${contract.registryKey}: ${rollbackErr}. ` +
        `This contract's pause state must be reconciled manually.`
      );
    }
  }
  if (failures.length > 0) {
    throw new Error(
      `Rollback incomplete; manual reconciliation required for: ${failures.join(", ")}`
    );
  }
}

propagatePause(desiredPaused);
