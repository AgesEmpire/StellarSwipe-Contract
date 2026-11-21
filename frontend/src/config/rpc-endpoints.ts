import endpoints from "../../../config/rpc_endpoints.json";

export type StellarNetworkName = "testnet" | "mainnet";

export interface RpcEndpointConfig {
  primary_rpc: string;
  fallback_rpc: string;
  horizon_url: string;
  network_passphrase: string;
}

type RpcEndpointMap = Record<StellarNetworkName, RpcEndpointConfig>;

const RPC_ENDPOINTS = endpoints as RpcEndpointMap;

function normalizeNetworkName(value: string | undefined): StellarNetworkName {
  return value?.toLowerCase() === "mainnet" ? "mainnet" : "testnet";
}

export function getSelectedStellarNetwork(): StellarNetworkName {
  return normalizeNetworkName(process.env.NEXT_PUBLIC_STELLAR_NETWORK);
}

export function getRpcEndpointConfig(
  network: StellarNetworkName = getSelectedStellarNetwork()
): RpcEndpointConfig {
  return RPC_ENDPOINTS[network];
}

export function getRpcCandidates(
  network: StellarNetworkName = getSelectedStellarNetwork()
): string[] {
  const config = getRpcEndpointConfig(network);
  return [config.primary_rpc, config.fallback_rpc].filter(
    (endpoint, index, all) => all.indexOf(endpoint) === index
  );
}

export function getNetworkMetadata(
  network: StellarNetworkName = getSelectedStellarNetwork()
): {
  network: StellarNetworkName;
  rpc: string;
  fallbackRpc: string;
  horizonUrl: string;
  networkPassphrase: string;
} {
  const config = getRpcEndpointConfig(network);
  return {
    network,
    rpc: config.primary_rpc,
    fallbackRpc: config.fallback_rpc,
    horizonUrl: config.horizon_url,
    networkPassphrase: config.network_passphrase,
  };
}
