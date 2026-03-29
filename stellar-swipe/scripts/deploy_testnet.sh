#!/usr/bin/env bash
# Deploy and initialize StellarSwipe Soroban contracts on testnet in dependency order.
#
# Logical order (see deployments/testnet.json keys):
#   common types  → no WASM (stellar_swipe_common is a library only)
#   StakeVault    → governance package
#   SignalRegistry → signal_registry package
#   FeeCollector  → oracle package
#   UserPortfolio → auto_trade package
#   TradeExecutor → bridge package (optional; see DEPLOY_TRADE_EXECUTOR)
#
# Requirements: stellar CLI, jq, built WASM (release or optimized).
#
# Required env (never commit secrets):
#   STELLAR_SOURCE_ACCOUNT   Secret seed (S...) / identity name / key as accepted by
#                            `stellar contract deploy --source-account` (same as STELLAR_ACCOUNT).
#   STELLAR_ADMIN_ADDRESS    StrKey (G...) used as contract admin in initialize().
#                            Must match the public key of STELLAR_SOURCE_ACCOUNT for
#                            governance (stake_vault) — initialize() calls admin.require_auth().
#
# Optional:
#   STELLAR_NETWORK            default: testnet
#   STELLAR_RPC_URL            Soroban RPC (overrides network default if set)
#   STELLAR_NETWORK_PASSPHRASE default: Test SDF Network ; September 2015
#   WASM_DIR                   default: target/wasm32-unknown-unknown/release
#   ROOT                       workspace root (parent of stellar-swipe); auto-detected
#   DEPLOY_TRADE_EXECUTOR      default 1; set 0 to skip bridge (no #[contract] on some branches)
#   GOVERNANCE_INIT_SKIP       set 1 to deploy governance WASM but skip initialize (manual CLI)
#   RECIPIENT_TEAM, RECIPIENT_EARLY_INVESTORS, ... (G...) override DistributionRecipients;
#                            default: all STELLAR_ADMIN_ADDRESS
#
# Idempotent: reuses contract_id from deployments/testnet.json and skips completed steps.
#
# Evidence log for PRs:
#   ./scripts/deploy_testnet.sh 2>&1 | tee deployments/testnet-deploy.log

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
SWIPE="$(cd "$SCRIPT_DIR/.." && pwd)"
STATE="${DEPLOY_STATE:-$ROOT/deployments/testnet.json}"
WASM_DIR="${WASM_DIR:-$SWIPE/target/wasm32-unknown-unknown/release}"
NET="${STELLAR_NETWORK:-testnet}"
DEPLOY_TRADE_EXECUTOR="${DEPLOY_TRADE_EXECUTOR:-1}"

die() { echo "error: $*" >&2; exit 1; }

command -v stellar >/dev/null || die "stellar CLI not found (install stellar-cli)"
command -v jq >/dev/null || die "jq not found"

[[ -n "${STELLAR_SOURCE_ACCOUNT:-${STELLAR_ACCOUNT:-}}" ]] || die "set STELLAR_SOURCE_ACCOUNT or STELLAR_ACCOUNT (signing key / identity)"
SOURCE="${STELLAR_SOURCE_ACCOUNT:-${STELLAR_ACCOUNT}}"

[[ -n "${STELLAR_ADMIN_ADDRESS:-}" ]] || die "set STELLAR_ADMIN_ADDRESS (G... admin strkey)"

ADMIN="$STELLAR_ADMIN_ADDRESS"

export STELLAR_NETWORK="$NET"
[[ -n "${STELLAR_RPC_URL:-}" ]] && export STELLAR_RPC_URL
[[ -n "${STELLAR_NETWORK_PASSPHRASE:-}" ]] || export STELLAR_NETWORK_PASSPHRASE="Test SDF Network ; September 2015"

mkdir -p "$(dirname "$STATE")"
if [[ ! -f "$STATE" ]]; then
  jq -n \
    --arg net "$NET" \
    --arg rpc "${STELLAR_RPC_URL:-}" \
    --arg ph "${STELLAR_NETWORK_PASSPHRASE}" \
    '{
      network: $net,
      rpc_url: $rpc,
      network_passphrase: $ph,
      note: "common/stellar_swipe_common is library-only; no deploy",
      contracts: {}
    }' >"$STATE"
fi

# Merge runtime network info
tmp=$(mktemp)
jq --arg net "$NET" --arg rpc "${STELLAR_RPC_URL:-}" --arg ph "$STELLAR_NETWORK_PASSPHRASE" \
  '.network = $net | .network_passphrase = $ph | .rpc_url = ($rpc // .rpc_url)' "$STATE" >"$tmp" && mv "$tmp" "$STATE"

rpc_flags=()
[[ -n "${STELLAR_RPC_URL:-}" ]] && rpc_flags+=(--rpc-url "$STELLAR_RPC_URL")
rpc_flags+=(--network-passphrase "$STELLAR_NETWORK_PASSPHRASE")

get_cid() {
  local logical="$1"
  jq -r --arg k "$logical" '.contracts[$k].contract_id // empty' "$STATE"
}

set_contract_meta() {
  local logical="$1" package="$2" cid="$3"
  local tmp
  tmp=$(mktemp)
  jq --arg k "$logical" --arg p "$package" --arg id "$cid" \
    '.contracts[$k] = (.contracts[$k] // {}) | .contracts[$k].package = $p | .contracts[$k].contract_id = $id' "$STATE" >"$tmp"
  mv "$tmp" "$STATE"
}

mark_initialized() {
  local logical="$1"
  local tmp
  tmp=$(mktemp)
  jq --arg k "$logical" '.contracts[$k].initialized = true' "$STATE" >"$tmp"
  mv "$tmp" "$STATE"
}

is_initialized_flag() {
  local logical="$1"
  [[ "$(jq -r --arg k "$logical" '.contracts[$k].initialized // false' "$STATE")" == "true" ]]
}

deploy_if_needed() {
  local logical="$1" package="$2"
  local wasm="$WASM_DIR/${package}.wasm"
  local existing cid out

  [[ -f "$wasm" ]] || die "missing WASM: $wasm (build with: cd stellar-swipe && cargo build --workspace --target wasm32-unknown-unknown --release)"

  existing="$(get_cid "$logical")"
  if [[ -n "$existing" ]]; then
    echo "==> $logical ($package): reuse $existing"
    return 0
  fi

  echo "==> $logical ($package): deploy $wasm"
  out="$(stellar contract deploy \
    --wasm "$wasm" \
    --source-account "$SOURCE" \
    --network "$NET" \
    "${rpc_flags[@]}" 2>&1)" || die "deploy failed: $out"
  echo "$out"
  cid="$(echo "$out" | grep -oE 'C[2-7A-Z]{55}' | tail -1 || true)"
  [[ -n "$cid" ]] || cid="$(echo "$out" | tail -n1 | tr -d '[:space:]')"
  [[ "$cid" =~ ^C[2-7A-Z]{55}$ ]] || die "could not parse contract id from deploy output (got: ${cid:-empty})"
  set_contract_meta "$logical" "$package" "$cid"
  echo "    contract_id=$cid"
}

# --- initialize steps ---

init_signal_registry() {
  local logical=signal_registry
  local cid
  cid="$(get_cid "$logical")"
  [[ -n "$cid" ]] || die "signal_registry not deployed"
  is_initialized_flag "$logical" && return 0
  echo "==> initialize signal_registry"
  stellar contract invoke \
    --id "$cid" \
    --source-account "$SOURCE" \
    --network "$NET" \
    "${rpc_flags[@]}" \
    -- \
    initialize \
    --admin "$ADMIN"
  mark_initialized "$logical"
}

init_oracle() {
  local logical=fee_collector
  local cid
  cid="$(get_cid "$logical")"
  [[ -n "$cid" ]] || die "oracle (fee_collector) not deployed"
  is_initialized_flag "$logical" && return 0
  echo "==> initialize oracle (fee_collector)"
  # Asset: XLM native — code "XLM", no issuer (CLI accepts void / null for Option::None)
  # Override with ORACLE_BASE_CURRENCY_JSON if your stellar-cli expects different ScVal JSON.
  local asset_json="${ORACLE_BASE_CURRENCY_JSON:-{\"code\":\"XLM\",\"issuer\":null}}"
  stellar contract invoke \
    --id "$cid" \
    --source-account "$SOURCE" \
    --network "$NET" \
    "${rpc_flags[@]}" \
    -- \
    initialize \
    --admin "$ADMIN" \
    --base_currency "$asset_json"
  mark_initialized "$logical"
}

init_auto_trade() {
  local logical=user_portfolio
  local cid
  cid="$(get_cid "$logical")"
  [[ -n "$cid" ]] || die "auto_trade (user_portfolio) not deployed"
  is_initialized_flag "$logical" && return 0
  echo "==> initialize auto_trade (user_portfolio)"
  stellar contract invoke \
    --id "$cid" \
    --source-account "$SOURCE" \
    --network "$NET" \
    "${rpc_flags[@]}" \
    -- \
    initialize \
    --admin "$ADMIN"
  mark_initialized "$logical"
}

init_governance() {
  local logical=stake_vault
  local cid
  cid="$(get_cid "$logical")"
  [[ -n "$cid" ]] || die "governance (stake_vault) not deployed"
  is_initialized_flag "$logical" && return 0
  if [[ "${GOVERNANCE_INIT_SKIP:-0}" == "1" ]]; then
    echo "==> skip governance initialize (GOVERNANCE_INIT_SKIP=1)"
    return 0
  fi

  local rt="${RECIPIENT_TEAM:-$ADMIN}"
  local re="${RECIPIENT_EARLY_INVESTORS:-$ADMIN}"
  local rc="${RECIPIENT_COMMUNITY_REWARDS:-$ADMIN}"
  local rtr="${RECIPIENT_TREASURY:-$ADMIN}"
  local rp="${RECIPIENT_PUBLIC_SALE:-$ADMIN}"
  local supply="${GOVERNANCE_TOTAL_SUPPLY:-1000000000000000}"

  echo "==> initialize governance (stake_vault) — recipients default to admin (testnet only)"
  # Nested struct flags (stellar-cli); if this fails on your CLI version, set GOVERNANCE_INIT_SKIP=1 and invoke manually.
  stellar contract invoke \
    --id "$cid" \
    --source-account "$SOURCE" \
    --network "$NET" \
    "${rpc_flags[@]}" \
    -- \
    initialize \
    --admin "$ADMIN" \
    --name "StellarSwipe Gov" \
    --symbol "SSG" \
    --decimals 7 \
    --total_supply "$supply" \
    --recipients.team "$rt" \
    --recipients.early_investors "$re" \
    --recipients.community_rewards "$rc" \
    --recipients.treasury "$rtr" \
    --recipients.public_sale "$rp" || die "governance initialize failed; try GOVERNANCE_INIT_SKIP=1 and run invoke manually (see script header)"
  mark_initialized "$logical"
}

init_bridge() {
  local logical=trade_executor
  local cid
  cid="$(get_cid "$logical")"
  [[ -n "$cid" ]] || return 0
  is_initialized_flag "$logical" && return 0
  echo "==> trade_executor (bridge): probe with health_check (deploy-only if it fails)"
  if stellar contract invoke \
    --id "$cid" \
    --source-account "$SOURCE" \
    --network "$NET" \
    "${rpc_flags[@]}" \
    --send=no \
    -- \
    health_check >/dev/null 2>&1; then
    echo "    bridge exposes health_check; no separate initialize required"
  else
    echo "    (optional) add initialize to bridge contract or set DEPLOY_TRADE_EXECUTOR=0"
  fi
  mark_initialized "$logical"
}

echo "Using STATE=$STATE WASM_DIR=$WASM_DIR NETWORK=$NET"

# Order: StakeVault → SignalRegistry → FeeCollector → UserPortfolio → TradeExecutor
deploy_if_needed stake_vault governance
deploy_if_needed signal_registry signal_registry
deploy_if_needed fee_collector oracle
deploy_if_needed user_portfolio auto_trade

if [[ "$DEPLOY_TRADE_EXECUTOR" == "1" ]]; then
  deploy_if_needed trade_executor bridge
else
  echo "==> skip trade_executor (bridge) DEPLOY_TRADE_EXECUTOR=0"
fi

init_governance
init_signal_registry
init_oracle
init_auto_trade
if [[ "$DEPLOY_TRADE_EXECUTOR" == "1" ]]; then
  init_bridge
fi

echo ""
echo "Wrote $STATE"
cat "$STATE"
echo ""
echo "Done. Save logs with: $0 2>&1 | tee deployments/testnet-deploy.log"
