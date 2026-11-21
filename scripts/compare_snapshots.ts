#!/usr/bin/env tsx
/**
 * compare_snapshots.ts — diff two contract state snapshots produced by
 * snapshot_state.ts and report added, removed, and changed storage entries.
 *
 * Usage:
 *   npx tsx scripts/compare_snapshots.ts <before.json> <after.json>
 *
 * Both files must be snapshot JSON files in the format emitted by
 * snapshot_state.ts.  The script exits non-zero when any differences are
 * found so it can be used as a CI gate or a pre-deploy safety check.
 *
 * Exit codes:
 *   0  Snapshots are identical.
 *   1  Differences found (or bad arguments / unreadable files).
 */

import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ── Types ─────────────────────────────────────────────────────────────────────

interface SnapshotEntry {
  lastModifiedLedgerSeq: number;
  liveUntilLedgerSeq?: number;
  key: unknown;
  value: unknown;
  rawKeyXdr: string;
  rawValueXdr: string;
}

interface Snapshot {
  latestLedger: number;
  entries: SnapshotEntry[];
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function usage(): never {
  console.error(
    "Usage: npx tsx scripts/compare_snapshots.ts <before.json> <after.json>"
  );
  process.exit(1);
}

function loadSnapshot(filePath: string): Snapshot {
  const resolved = path.resolve(filePath);
  let raw: string;
  try {
    raw = fs.readFileSync(resolved, "utf8");
  } catch (err) {
    console.error(`Error reading ${resolved}:`, (err as Error).message);
    process.exit(1);
  }
  try {
    return JSON.parse(raw) as Snapshot;
  } catch (err) {
    console.error(`Error parsing ${resolved}:`, (err as Error).message);
    process.exit(1);
  }
}

/** Stable string key derived from the raw XDR key so map lookups are exact. */
function entryKey(e: SnapshotEntry): string {
  return e.rawKeyXdr;
}

function stableJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function summarize(label: string, count: number): void {
  if (count === 0) return;
  console.log(`  ${label}: ${count}`);
}

// ── Main ──────────────────────────────────────────────────────────────────────

function main(): void {
  const [beforePath, afterPath] = process.argv.slice(2);
  if (!beforePath || !afterPath) {
    usage();
  }

  const before = loadSnapshot(beforePath!);
  const after = loadSnapshot(afterPath!);

  console.log(`Comparing snapshots:`);
  console.log(`  before: ${beforePath} (ledger ${before.latestLedger})`);
  console.log(`  after:  ${afterPath}  (ledger ${after.latestLedger})`);
  console.log();

  // Build maps keyed by rawKeyXdr for O(1) lookup.
  const beforeMap = new Map<string, SnapshotEntry>();
  for (const entry of before.entries) {
    beforeMap.set(entryKey(entry), entry);
  }
  const afterMap = new Map<string, SnapshotEntry>();
  for (const entry of after.entries) {
    afterMap.set(entryKey(entry), entry);
  }

  const added: SnapshotEntry[] = [];
  const removed: SnapshotEntry[] = [];
  const changed: Array<{ before: SnapshotEntry; after: SnapshotEntry }> = [];
  const unchanged: number[] = [];

  // Entries in after: new or changed.
  for (const [k, aEntry] of afterMap) {
    const bEntry = beforeMap.get(k);
    if (!bEntry) {
      added.push(aEntry);
    } else if (aEntry.rawValueXdr !== bEntry.rawValueXdr) {
      changed.push({ before: bEntry, after: aEntry });
    } else {
      unchanged.push(1);
    }
  }

  // Entries only in before: removed.
  for (const [k, bEntry] of beforeMap) {
    if (!afterMap.has(k)) {
      removed.push(bEntry);
    }
  }

  const hasDiff = added.length > 0 || removed.length > 0 || changed.length > 0;

  if (!hasDiff) {
    console.log("✓ Snapshots are identical — no state changes detected.");
    process.exit(0);
  }

  // ── Report ─────────────────────────────────────────────────────────────────

  console.log("State diff summary:");
  summarize("added  ", added.length);
  summarize("removed", removed.length);
  summarize("changed", changed.length);
  console.log();

  if (added.length > 0) {
    console.log("── ADDED entries ─────────────────────────────────────────────");
    for (const entry of added) {
      console.log(`  key:   ${JSON.stringify(entry.key)}`);
      console.log(`  value: ${stableJson(entry.value)}`);
      console.log();
    }
  }

  if (removed.length > 0) {
    console.log("── REMOVED entries ───────────────────────────────────────────");
    for (const entry of removed) {
      console.log(`  key:   ${JSON.stringify(entry.key)}`);
      console.log(`  value: ${stableJson(entry.value)}`);
      console.log();
    }
  }

  if (changed.length > 0) {
    console.log("── CHANGED entries ───────────────────────────────────────────");
    for (const { before: b, after: a } of changed) {
      console.log(`  key:    ${JSON.stringify(a.key)}`);
      console.log(`  before: ${stableJson(b.value)}`);
      console.log(`  after:  ${stableJson(a.value)}`);
      console.log();
    }
  }

  process.exit(1);
}

main();
