import assert from "node:assert";
import { describe, it } from "node:test";
import { Simulator, SignalError, PositionStatus } from "../../scripts/simulator/index.ts";

function setupSimulator() {
  const sim = new Simulator();
  const admin = sim.createAddress("admin");
  const oracle = sim.createAddress("oracle");
  const user = sim.createAddress("user");
  const provider = sim.createAddress("provider");

  sim.oracle.initialize(admin);
  sim.oracle.setPrice(1, 1000n);
  sim.userPortfolio.initialize(admin, oracle);
  sim.signalRegistry.createSignal(provider, sim.now + 1000);
  return { sim, admin, user, provider };
}

describe("Regression test suite", () => {
  it("test_issue_001_zero_amount_trade", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const oracle = sim.createAddress("oracle");
    const user = sim.createAddress("user");
    const provider = sim.createAddress("provider");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, oracle);
    sim.setBalance(user, "USD", 10_000n);

    const signalId = sim.signalRegistry.createSignal(provider, sim.now + 10);

    assert.throws(
      () => sim.tradeExecutor.executeCopyTrade(user, "USD", 0n, signalId),
      /invalid amount/
    );
  });

  it("test_issue_002_expired_signal_copy", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const oracle = sim.createAddress("oracle");
    const user = sim.createAddress("user");
    const provider = sim.createAddress("provider");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, oracle);
    sim.setBalance(user, "USD", 1000n);
    const signalId = sim.signalRegistry.createSignal(provider, sim.now + 1);
    sim.advanceTime(10);

    assert.throws(
      () => sim.tradeExecutor.executeCopyTrade(user, "USD", 100n, signalId),
      /SignalExpired/
    );
  });

  it("test_issue_003_double_close_attempt", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const oracle = sim.createAddress("oracle");
    const user = sim.createAddress("user");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, oracle);
    const positionId = sim.userPortfolio.openPosition(user, 100n, 1_000n);
    sim.userPortfolio.closePosition(user, positionId, 100n, 110n, 1, sim.createAddress("provider"), 1);

    assert.throws(
      () => sim.userPortfolio.closePosition(user, positionId, 50n, 105n, 1, sim.createAddress("provider"), 2),
      /PositionAlreadyClosed/
    );
  });

  it("test_issue_004_reentrancy_guard", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const oracle = sim.createAddress("oracle");
    const user = sim.createAddress("user");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, oracle);
    const positionId = sim.userPortfolio.openPosition(user, 100n, 1_000n);
    sim.userPortfolio.closePosition(user, positionId, 100n, 110n, 1, sim.createAddress("provider"), 1);

    // A second close should fail immediately instead of allowing a second 'closing' step.
    assert.throws(
      () => sim.userPortfolio.closePosition(user, positionId, 200n, 120n, 1, sim.createAddress("provider"), 2),
      /PositionAlreadyClosed/
    );
  });

  it("test_issue_005_missing_trade_executor", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const user = sim.createAddress("user");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, sim.createAddress("oracle"));
    const positionId = sim.userPortfolio.openPosition(user, 100n, 1_000n);

    assert.throws(
      () => sim.userPortfolio.closePositionKeeper(sim.createAddress("attacker"), user, positionId, 1),
      /trade executor not set/
    );
  });

  it("test_issue_006_unauthorized_fee_claim", () => {
    const sim = new Simulator();
    const trader = sim.createAddress("trader");
    const provider = sim.createAddress("provider");
    sim.setBalance(trader, "USD", 1_000n);
    sim.feeCollector.collectFee(trader, "USD", 100n);

    assert.throws(
      () => sim.feeCollector.claimFees(provider, "USD", sim.createAddress("attacker")),
      /unauthorized fee claim/
    );
  });

  it("test_issue_007_closed_position_cannot_reopen", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const oracle = sim.createAddress("oracle");
    const user = sim.createAddress("user");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, oracle);
    const positionId = sim.userPortfolio.openPosition(user, 100n, 1_000n);
    sim.userPortfolio.closePosition(user, positionId, 50n, 105n, 1, sim.createAddress("provider"), 1);

    const position = sim.positions.get(positionId);
    assert.strictEqual(position?.status, PositionStatus.Closed);
  });

  it("test_issue_008_expired_signal_nonblocking_new_signal", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const oracle = sim.createAddress("oracle");
    const user = sim.createAddress("user");
    const provider = sim.createAddress("provider");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, oracle);
    sim.subscribeToProvider(user, provider);

    const oldSignal = sim.signalRegistry.createSignal(provider, sim.now + 1);
    sim.advanceTime(10);
    const freshSignal = sim.signalRegistry.createSignal(provider, sim.now + 100);

    assert.throws(
      () => sim.signalRegistry.getSignalForViewer(oldSignal, user),
      /SignalExpired/
    );
    const signal = sim.signalRegistry.getSignalForViewer(freshSignal, user);
    assert.strictEqual(signal.signalId, freshSignal);
  });

  it("test_issue_009_position_limit_enforced", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const oracle = sim.createAddress("oracle");
    const user = sim.createAddress("user");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, oracle);
    sim.setBalance(user, "USD", 10_000n);
    const provider = sim.createAddress("provider");
    const signalId = sim.signalRegistry.createSignal(provider, sim.now + 1000);

    for (let i = 0; i < 5; i += 1) {
      sim.tradeExecutor.executeCopyTrade(user, "USD", 100n, signalId, 5);
    }

    assert.throws(
      () => sim.tradeExecutor.executeCopyTrade(user, "USD", 100n, signalId, 5),
      /PositionLimitReached/
    );
  });

  it("test_issue_010_kyc_required_mode", () => {
    const sim = new Simulator();
    const admin = sim.createAddress("admin");
    const user = sim.createAddress("user");
    sim.oracle.initialize(admin);
    sim.oracle.setPrice(1, 1000n);
    sim.userPortfolio.initialize(admin, sim.createAddress("oracle"));
    sim.userPortfolio.setKycRequiredMode(admin, true);

    assert.throws(
      () => sim.userPortfolio.openPosition(user, 100n, 1_000n),
      /KYC verification required to open a position/
    );
  });
});
