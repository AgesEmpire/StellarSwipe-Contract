export type Address = string;
export type Token = string;
export type AssetPair = number;

export enum PositionStatus {
  Open = "Open",
  Closing = "Closing",
  Closed = "Closed",
}

export interface Position {
  positionId: number;
  owner: Address;
  entryPrice: bigint;
  amount: bigint;
  status: PositionStatus;
  realizedPnl: bigint;
}

export interface Signal {
  signalId: number;
  provider: Address;
  expiry: number;
  active: boolean;
}

export enum SignalError {
  SignalExpired = "SignalExpired",
  SignalNotFound = "SignalNotFound",
}

export class Simulator {
  private addressCounter = 0;
  public now = 1_000_000;
  public oracle = new OracleContract(this);
  public userPortfolio = new UserPortfolioContract(this);
  public tradeExecutor = new TradeExecutorContract(this);
  public feeCollector = new FeeCollectorContract(this);
  public signalRegistry = new SignalRegistryContract(this);

  public balances = new Map<string, bigint>();
  public positions = new Map<number, Position>();
  public userPositions = new Map<Address, Set<number>>();
  public nextPositionId = 1;
  public tradeExecutorAddress: Address | null = null;
  public admin: Address | null = null;
  public signalSubscriptions = new Map<Address, Set<Address>>();
  public signals = new Map<number, Signal>();
  public nextSignalId = 1;
  public feePools = new Map<Token, bigint>();

  constructor() {
    this.tradeExecutorAddress = "trade_executor";
  }

  public createAddress(prefix: string): Address {
    this.addressCounter += 1;
    return `${prefix}-${this.addressCounter}`;
  }

  public setBalance(user: Address, token: Token, amount: bigint): void {
    this.balances.set(this.balanceKey(user, token), amount);
  }

  public getBalance(user: Address, token: Token): bigint {
    return this.balances.get(this.balanceKey(user, token)) ?? 0n;
  }

  public advanceTime(seconds: number): void {
    this.now += seconds;
  }

  public subscribeToProvider(user: Address, provider: Address): void {
    const set = this.signalSubscriptions.get(user) ?? new Set<Address>();
    set.add(provider);
    this.signalSubscriptions.set(user, set);
  }

  public isSubscribed(user: Address, provider: Address): boolean {
    return this.signalSubscriptions.get(user)?.has(provider) ?? false;
  }

  private balanceKey(user: Address, token: Token): string {
    return `${user}:${token}`;
  }
}

export class OracleContract {
  public address: Address;
  private prices = new Map<AssetPair, bigint>();
  private sim: Simulator;

  constructor(sim: Simulator) {
    this.sim = sim;
    this.address = "oracle";
  }

  public initialize(admin: Address): void {
    this.sim.admin = admin;
  }

  public setPrice(assetPair: AssetPair, price: bigint): void {
    this.prices.set(assetPair, price);
  }

  public getPrice(assetPair: AssetPair): bigint {
    const price = this.prices.get(assetPair);
    if (price === undefined) {
      throw new Error("price unavailable");
    }
    return price;
  }
}

export class UserPortfolioContract {
  private sim: Simulator;
  private oracleAddress: Address | null = null;
  private tradeExecutor: Address | null = null;
  private kycRequired = false;

  constructor(sim: Simulator) {
    this.sim = sim;
  }

  public initialize(admin: Address, oracle: Address): void {
    if (this.oracleAddress !== null) {
      throw new Error("user portfolio already initialized");
    }
    if (this.sim.admin === null) {
      this.sim.admin = admin;
    } else if (this.sim.admin !== admin) {
      throw new Error("inconsistent admin");
    }
    this.oracleAddress = oracle;
  }

  public setTradeExecutor(caller: Address, tradeExecutor: Address): void {
    this.requireAdmin(caller);
    this.tradeExecutor = tradeExecutor;
  }

  public setKycRequiredMode(caller: Address, required: boolean): void {
    this.requireAdmin(caller);
    this.kycRequired = required;
  }

  public openPosition(user: Address, entryPrice: bigint, amount: bigint): number {
    if (entryPrice <= 0n || amount <= 0n) {
      throw new Error("invalid entry_price or amount");
    }
    if (this.kycRequired) {
      throw new Error("KYC verification required to open a position");
    }

    const id = this.sim.nextPositionId++;
    const position: Position = {
      positionId: id,
      owner: user,
      entryPrice,
      amount,
      status: PositionStatus.Open,
      realizedPnl: 0n,
    };
    this.sim.positions.set(id, position);

    const set = this.sim.userPositions.get(user) ?? new Set<number>();
    set.add(id);
    this.sim.userPositions.set(user, set);

    return id;
  }

  public validateAndRecord(user: Address, maxPositions: number): void {
    const openCount = this.openPositionCount(user);
    if (openCount >= maxPositions) {
      throw new Error("PositionLimitReached");
    }
  }

  public hasPosition(user: Address, positionId: number): boolean {
    const set = this.sim.userPositions.get(user);
    return set?.has(positionId) ?? false;
  }

  public closePosition(
    user: Address,
    positionId: number,
    realizedPnl: bigint,
    exitPrice: bigint,
    assetPair: AssetPair,
    signalProvider: Address,
    signalId: number
  ): void {
    if (!this.hasPosition(user, positionId)) {
      throw new Error("position not found for user");
    }
    const position = this.sim.positions.get(positionId);
    if (!position) {
      throw new Error("position missing");
    }
    if (position.status !== PositionStatus.Open) {
      throw new Error("PositionAlreadyClosed");
    }

    position.status = PositionStatus.Closing;
    this.sim.positions.set(positionId, position);

    position.status = PositionStatus.Closed;
    position.realizedPnl = realizedPnl;
    this.sim.positions.set(positionId, position);
  }

  public closePositionKeeper(caller: Address, user: Address, positionId: number, assetPair: AssetPair): void {
    if (this.tradeExecutor === null) {
      throw new Error("trade executor not set");
    }
    if (caller !== this.tradeExecutor) {
      throw new Error("unauthorized keeper call");
    }
    this.closePosition(user, positionId, 0n, 0n, assetPair, caller, 0);
  }

  public openPositionCount(user: Address): number {
    const set = this.sim.userPositions.get(user);
    if (!set) {
      return 0;
    }
    let count = 0;
    for (const positionId of set) {
      const position = this.sim.positions.get(positionId);
      if (position && position.status === PositionStatus.Open) {
        count += 1;
      }
    }
    return count;
  }

  private requireAdmin(caller: Address): void {
    if (caller !== this.sim.admin) {
      throw new Error("unauthorized");
    }
  }
}

export class TradeExecutorContract {
  private sim: Simulator;

  constructor(sim: Simulator) {
    this.sim = sim;
  }

  public executeCopyTrade(user: Address, token: Token, amount: bigint, signalId: number, maxPositions = 5): number {
    if (amount <= 0n) {
      throw new Error("invalid amount");
    }
    const signal = this.sim.signals.get(signalId);
    if (!signal) {
      throw new Error(SignalError.SignalNotFound);
    }
    if (signal.expiry <= this.sim.now || !signal.active) {
      throw new Error(SignalError.SignalExpired);
    }

    this.sim.userPortfolio.validateAndRecord(user, maxPositions);

    const balance = this.sim.getBalance(user, token);
    if (balance < amount) {
      throw new Error("insufficient balance");
    }
    this.sim.setBalance(user, token, balance - amount);

    return this.sim.userPortfolio.openPosition(user, 1n, amount);
  }
}

export class FeeCollectorContract {
  private sim: Simulator;

  constructor(sim: Simulator) {
    this.sim = sim;
  }

  public collectFee(trader: Address, token: Token, feeAmount: bigint): void {
    if (feeAmount <= 0n) {
      throw new Error("invalid fee amount");
    }
    const balance = this.sim.getBalance(trader, token);
    if (balance < feeAmount) {
      throw new Error("insufficient balance");
    }
    this.sim.setBalance(trader, token, balance - feeAmount);
    const pool = this.sim.feePools.get(token) ?? 0n;
    this.sim.feePools.set(token, pool + feeAmount);
  }

  public claimFees(provider: Address, token: Token, caller: Address): void {
    if (provider !== caller) {
      throw new Error("unauthorized fee claim");
    }
    const pool = this.sim.feePools.get(token) ?? 0n;
    if (pool <= 0n) {
      throw new Error("no fees available");
    }
    this.sim.setBalance(provider, token, this.sim.getBalance(provider, token) + pool);
    this.sim.feePools.set(token, 0n);
  }
}

export class SignalRegistryContract {
  private sim: Simulator;

  constructor(sim: Simulator) {
    this.sim = sim;
  }

  public createSignal(provider: Address, expiry: number): number {
    const id = this.sim.nextSignalId++;
    this.sim.signals.set(id, {
      signalId: id,
      provider,
      expiry,
      active: true,
    });
    return id;
  }

  public getSignalForViewer(signalId: number, viewer: Address): Signal {
    const signal = this.sim.signals.get(signalId);
    if (!signal) {
      throw new Error(SignalError.SignalNotFound);
    }
    if (signal.expiry <= this.sim.now) {
      throw new Error(SignalError.SignalExpired);
    }
    if (!this.sim.isSubscribed(viewer, signal.provider)) {
      throw new Error("subscription required");
    }
    return signal;
  }

  public cleanupExpiredSignals(limit: number): { processed: number; expired: number } {
    let processed = 0;
    let expired = 0;
    for (const signal of Array.from(this.sim.signals.values())) {
      if (processed >= limit) {
        break;
      }
      if (signal.expiry <= this.sim.now && signal.active) {
        signal.active = false;
        this.sim.signals.set(signal.signalId, signal);
        expired += 1;
      }
      processed += 1;
    }
    return { processed, expired };
  }
}
