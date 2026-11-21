import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { Simulator } from "./simulator/index.js";

function generatedAmounts(count: number): bigint[] {
  let seed = 0x5eedn;
  const amounts: bigint[] = [0n, 1n, 2n ** 63n - 1n];
  for (let index = amounts.length; index < count; index += 1) {
    seed = (seed * 6364136223846793005n + 1442695040888963407n) & ((1n << 64n) - 1n);
    amounts.push(seed % 1_000_000_000_000n);
  }
  return amounts;
}

describe("reward and fee conservation", () => {
  it("conserves every generated funded amount and never allocates negative rewards", () => {
    for (const amount of generatedAmounts(256)) {
      const sim = new Simulator();
      const trader = sim.createAddress("trader");
      const provider = sim.createAddress("provider");
      sim.setBalance(trader, "USD", amount);

      if (amount === 0n) {
        assert.throws(() => sim.feeCollector.collectFee(trader, "USD", amount), /invalid fee amount/);
        continue;
      }

      sim.feeCollector.collectFee(trader, "USD", amount);
      assert.equal(sim.getBalance(trader, "USD") + (sim.feePools.get("USD") ?? 0n), amount);
      sim.feeCollector.claimFees(provider, "USD", provider);
      assert.equal(sim.getBalance(provider, "USD"), amount);
      assert.equal(sim.feePools.get("USD"), 0n);
      assert.ok(sim.getBalance(provider, "USD") >= 0n);
    }
  });

  it("rejects unauthorized claims without changing the funded pool", () => {
    const sim = new Simulator();
    const trader = sim.createAddress("trader");
    const provider = sim.createAddress("provider");
    const attacker = sim.createAddress("attacker");
    sim.setBalance(trader, "USD", 100n);
    sim.feeCollector.collectFee(trader, "USD", 100n);

    assert.throws(() => sim.feeCollector.claimFees(provider, "USD", attacker), /unauthorized fee claim/);
    assert.equal(sim.feePools.get("USD"), 100n);
    assert.equal(sim.getBalance(provider, "USD"), 0n);
  });
});