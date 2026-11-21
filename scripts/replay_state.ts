/**
 * Event replay → state reconstruction (Issue #882).
 *
 * `replay_events.ts` fetches and parses raw Soroban events. This module takes
 * that parsed stream and folds it into a coherent state snapshot: per-user
 * stakes, per-signal trade stats, collected fees and the latest oracle price.
 * The reducer is pure and deterministic, so a fixture of events always replays
 * to the same snapshot — which is what the fixture test asserts.
 *
 * CLI:
 *   npx tsx replay_state.ts sample_parsed_events_testnet.json
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export interface ParsedEvent {
  id: string;
  ledger: number;
  txHash: string;
  contractId: string;
  eventName: string;
  topicsNative: unknown[];
  dataNative: Record<string, string | number>;
}

export interface StateSnapshot {
  /** Ledger of the last event folded in. */
  lastLedger: number;
  /** Number of events applied (events with unknown names are skipped). */
  appliedEvents: number;
  /** Event names seen but not handled by any reducer. */
  unhandledEvents: string[];
  /** Staked balance per account. */
  stakes: Record<string, bigint>;
  /** Executed trade count and cumulative volume per signal. */
  signals: Record<string, { trades: number; volume: bigint }>;
  /** Total fees collected. */
  feesCollected: bigint;
  /** Most recent oracle price, or null if none was seen. */
  lastOraclePrice: bigint | null;
}

function emptySnapshot(): StateSnapshot {
  return {
    lastLedger: 0,
    appliedEvents: 0,
    unhandledEvents: [],
    stakes: {},
    signals: {},
    feesCollected: 0n,
    lastOraclePrice: null,
  };
}

function big(value: string | number | undefined): bigint {
  return value === undefined ? 0n : BigInt(value);
}

function str(value: string | number | undefined): string {
  return value === undefined ? "" : String(value);
}

/** Folds a single parsed event into `state`. Returns false if unhandled. */
function apply(state: StateSnapshot, event: ParsedEvent): boolean {
  const data = event.dataNative ?? {};

  switch (event.eventName) {
    case "staked": {
      const who = str(data.staker ?? data.user);
      state.stakes[who] = (state.stakes[who] ?? 0n) + big(data.amount);
      return true;
    }
    case "unstaked": {
      const who = str(data.staker ?? data.user);
      state.stakes[who] = (state.stakes[who] ?? 0n) - big(data.amount);
      return true;
    }
    case "trade_executed": {
      const signalId = str(data.signal_id);
      const entry = state.signals[signalId] ?? { trades: 0, volume: 0n };
      entry.trades += 1;
      entry.volume += big(data.volume);
      state.signals[signalId] = entry;
      return true;
    }
    case "fee_collected": {
      state.feesCollected += big(data.amount ?? data.fee);
      return true;
    }
    case "oracle_price_submitted": {
      state.lastOraclePrice = big(data.price);
      return true;
    }
    default:
      return false;
  }
}

/**
 * Replays parsed events into a state snapshot. Events are sorted by ledger
 * first so an out-of-order page from the RPC still replays deterministically.
 */
export function replay(events: ParsedEvent[]): StateSnapshot {
  const state = emptySnapshot();
  const unhandled = new Set<string>();

  const ordered = [...events].sort(
    (a, b) => a.ledger - b.ledger || a.id.localeCompare(b.id),
  );

  for (const event of ordered) {
    if (apply(state, event)) {
      state.appliedEvents += 1;
    } else {
      unhandled.add(event.eventName);
    }
    state.lastLedger = Math.max(state.lastLedger, event.ledger);
  }

  state.unhandledEvents = [...unhandled].sort();
  return state;
}

/** Reads a parsed-event dump (the shape written by `test_event_parsing.ts`). */
export function replayFile(path: string): StateSnapshot {
  const dump = JSON.parse(readFileSync(path, "utf8")) as { events: ParsedEvent[] };
  return replay(dump.events ?? []);
}

/** JSON-safe rendering of a snapshot (bigints become decimal strings). */
export function serialize(state: StateSnapshot): string {
  return JSON.stringify(
    state,
    (_key, value) => (typeof value === "bigint" ? value.toString() : value),
    2,
  );
}

function main(): void {
  const path = process.argv[2] ?? "sample_parsed_events_testnet.json";
  console.log(serialize(replayFile(path)));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
