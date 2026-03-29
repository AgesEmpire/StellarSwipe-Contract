#!/usr/bin/env bash
# Wrapper: Soroban workspace lives under stellar-swipe/.
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../stellar-swipe/scripts/build.sh" "$@"
