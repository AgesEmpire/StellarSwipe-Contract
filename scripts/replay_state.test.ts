/**
 * Fixture tests for the event replay reducer (Issue #882).
 *
 * Run: npx tsx --test replay_state.test.ts
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { replay, replayFile, serialize, type ParsedEvent } from "./replay_state.ts";

const FIXTURE = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "fixtures/replay_events_fixture.json",
);

test("empty event stream replays to an empty snapshot", () => {
  const state = replay([]);
  assert.equal(state.lastLedger, 0);
  assert.equal(state.appliedEvents, 0);
  assert.deepEqual(state.stakes, {});
  assert.equal(state.feesCollected, 0n);
  assert.equal(state.lastOraclePrice, null);
});

test("fixture replays to the known state snapshot", () => {
  const state = replayFile(FIXTURE);

  assert.equal(state.lastLedger, 108);
  assert.equal(state.appliedEvents, 8); // 9 events, badge_awarded is unhandled
  assert.deepEqual(state.stakes, { GALICE: 600n, GBOB: 2500n });
  assert.deepEqual(state.signals, { "7": { trades: 2, volume: 6500n } });
  assert.equal(state.feesCollected, 65n);
  assert.equal(state.lastOraclePrice, 1_050_000n);
});

test("unhandled event names are reported, not silently dropped", () => {
  const state = replayFile(FIXTURE);
  assert.deepEqual(state.unhandledEvents, ["badge_awarded"]);
});

test("out-of-order events replay deterministically", () => {
  const events = (JSON.parse(readFileSync(FIXTURE, "utf8")) as { events: ParsedEvent[] })
    .events;

  const forward = replay(events);
  const reversed = replay([...events].reverse());
  assert.equal(serialize(forward), serialize(reversed));
});

test("snapshot serializes bigints as decimal strings", () => {
  const state = replayFile(FIXTURE);
  const json = JSON.parse(serialize(state));
  assert.equal(json.stakes.GALICE, "600");
  assert.equal(json.feesCollected, "65");
});
