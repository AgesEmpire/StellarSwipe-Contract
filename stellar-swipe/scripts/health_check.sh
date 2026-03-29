#!/usr/bin/env bash
# Read-only health_check against deployed Soroban contracts (Stellar CLI).
# Set contract IDs and RPC; script exits non-zero if any probe fails.
set -euo pipefail

RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"

: "${SIGNAL_REGISTRY_ID:?Set SIGNAL_REGISTRY_ID}"
: "${ORACLE_ID:?Set ORACLE_ID}"
: "${GOVERNANCE_ID:?Set GOVERNANCE_ID}"
: "${AUTO_TRADE_ID:?Set AUTO_TRADE_ID}"
: "${BRIDGE_ID:?Set BRIDGE_ID}"

probe() {
  local name="$1"
  local id="$2"
  echo "==> $name ($id)"
  stellar contract invoke \
    --id "$id" \
    --rpc-url "$RPC_URL" \
    --network-passphrase "$NETWORK_PASSPHRASE" \
    --send=no \
    -- health_check
}

probe signal_registry "$SIGNAL_REGISTRY_ID"
probe oracle "$ORACLE_ID"
probe governance "$GOVERNANCE_ID"
probe auto_trade "$AUTO_TRADE_ID"
probe bridge "$BRIDGE_ID"

echo "All health_check calls succeeded."
