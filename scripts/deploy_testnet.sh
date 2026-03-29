#!/usr/bin/env bash
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../stellar-swipe/scripts/deploy_testnet.sh" "$@"
