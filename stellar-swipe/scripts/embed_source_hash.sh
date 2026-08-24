#!/usr/bin/env bash
# Compute a reproducible SHA-256 of the source snapshot used for a given
# build and export it as STELLAR_SOURCE_HASH so contractmeta! can embed it.
#
# Usage (called automatically by build.sh before cargo build):
#   source ./scripts/embed_source_hash.sh   # sets STELLAR_SOURCE_HASH in the
#                                            # caller's environment
#
# The hash covers:
#   - Every *.rs and *.toml file under stellar-swipe/ (excluding target/)
#   - Sorted by path for reproducibility across platforms
#
# Third-party verification:
#   1. Fetch the source archive that matches the commit recorded in the
#      deployed contract metadata (key "GitCommit").
#   2. Run this script from the stellar-swipe/ workspace root.
#   3. Compare STELLAR_SOURCE_HASH against the value read from the deployed
#      WASM with:  stellar contract inspect --wasm <file.wasm>
#
# CI enforcement: build.sh calls this script and will exit non-zero if the
# hash is empty, preventing a WASM from being shipped with missing metadata.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"

# Collect all tracked source and manifest files, sorted for reproducibility.
mapfile -t SOURCE_FILES < <(
  git -C "$WORKSPACE" ls-files \
    -- '*.rs' '*.toml' \
    ':!:target/**' \
  | sort
)

if [[ ${#SOURCE_FILES[@]} -eq 0 ]]; then
  echo "error: no source files found under $WORKSPACE" >&2
  exit 1
fi

# Feed each file's content through sha256sum, then hash the combined listing.
STELLAR_SOURCE_HASH=$(
  (
    for f in "${SOURCE_FILES[@]}"; do
      sha256sum "$WORKSPACE/$f"
    done
  ) | sha256sum | awk '{print $1}'
)

export STELLAR_SOURCE_HASH

# Also capture the git commit so contracts embed both for full traceability.
GIT_COMMIT=$(git -C "$WORKSPACE" rev-parse HEAD 2>/dev/null || echo "unknown")
export STELLAR_GIT_COMMIT="$GIT_COMMIT"

echo "Source hash : $STELLAR_SOURCE_HASH"
echo "Git commit  : $STELLAR_GIT_COMMIT"
