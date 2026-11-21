#!/usr/bin/env bash
# Update the committed instruction-budget baseline after an intentional change.
#
# Usage:
#   ./scripts/update_budget_baseline.sh
#
# This runs all workspace tests, captures BUDGET_METRIC lines from stdout,
# and feeds them to check_budget_baseline.py --update, which overwrites
# baselines/instruction_budget_baseline.json with the new measurements.
#
# After running, review the diff, then commit:
#   git add baselines/instruction_budget_baseline.json
#   git commit -m "chore: update instruction budget baseline — <reason>"
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

echo "Running tests to capture budget measurements..."
cargo test --workspace --all-targets 2>&1 \
  | tee /tmp/budget_test_output.txt \
  | python3 scripts/check_budget_baseline.py --update

echo ""
echo "Done. Review the diff and commit baselines/instruction_budget_baseline.json."
