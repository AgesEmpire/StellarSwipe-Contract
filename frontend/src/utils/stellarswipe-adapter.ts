import {
  getRpcCandidates,
  getRpcEndpointConfig,
  type RpcEndpointConfig,
} from "../config/rpc-endpoints";

export interface StellarSwipeStats {
  cash: number;
  incomeRate: number;
  boosts: number;
}

export type FetchErrorKind = "network" | "server";

export class FetchError extends Error {
  constructor(public kind: FetchErrorKind, message: string) {
    super(message);
    this.name = "FetchError";
  }
}

export class StellarSwipeHUDAdapter {
  private contractAddress: string;
  private network: RpcEndpointConfig;

  constructor(contractAddress: string, network: RpcEndpointConfig = getRpcEndpointConfig()) {
    this.contractAddress = contractAddress;
    this.network = network;
  }

  get rpcUrl(): string {
    return this.network.primary_rpc;
  }

  get fallbackRpcUrl(): string {
    return this.network.fallback_rpc;
  }

  get horizonUrl(): string {
    return this.network.horizon_url;
  }

  get networkPassphrase(): string {
    return this.network.network_passphrase;
  }

  private get rpcCandidates(): string[] {
    return getRpcCandidates(
      this.network.network_passphrase === getRpcEndpointConfig("mainnet").network_passphrase
        ? "mainnet"
        : "testnet"
    );
  }

  async fetchTycoonStats(): Promise<StellarSwipeStats> {
    const errors: string[] = [];

    for (const baseUrl of this.rpcCandidates) {
      try {
        const response = await fetch(
          `${baseUrl.replace(/\/+$/, "")}/contracts/${this.contractAddress}/stats`
        );
        if (!response.ok) {
          errors.push(`HTTP ${response.status}: ${response.statusText}`);
          continue;
        }

        const data = (await response.json()) as Partial<StellarSwipeStats> & {
          income_rate?: number;
          active_boosts?: number;
        };

        return {
          cash: data.cash ?? 0,
          incomeRate: data.incomeRate ?? data.income_rate ?? 0,
          boosts: data.boosts ?? data.active_boosts ?? 0,
        };
      } catch (error) {
        errors.push(error instanceof Error ? error.message : String(error));
      }
    }

    throw new FetchError(
      "network",
      errors.length > 0
        ? `Unable to reach the selected RPC endpoints: ${errors.join(" | ")}`
        : "Unable to reach the selected RPC endpoint."
    );
  }

  async batchFetchStats(requests: string[]): Promise<StellarSwipeStats[]> {
    const errors: string[] = [];

    for (const baseUrl of this.rpcCandidates) {
      try {
        const response = await fetch(`${baseUrl.replace(/\/+$/, "")}/batch`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ requests }),
        });

        if (!response.ok) {
          errors.push(`HTTP ${response.status}: ${response.statusText}`);
          continue;
        }

        return (await response.json()) as StellarSwipeStats[];
      } catch (error) {
        errors.push(error instanceof Error ? error.message : String(error));
      }
    }

    throw new FetchError(
      "network",
      errors.length > 0
        ? `Unable to reach the selected RPC endpoints: ${errors.join(" | ")}`
        : "Unable to reach the selected RPC endpoint."
    );
  }
}
