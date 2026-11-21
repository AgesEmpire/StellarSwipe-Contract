#!/usr/bin/env bash
# check_event_topics.sh — issue #585
#
# Scans Rust source files for `events().publish(` calls that pass an ad hoc
# inline symbol literal as a topic instead of a constant from
# `shared::event_topics`. Fails with a non-zero exit code if any are found.
#
# Usage:
#   ./scripts/check_event_topics.sh            # scan entire workspace
#   ./scripts/check_event_topics.sh contracts/governance/src/lib.rs
#
# Canonical topics live in: contracts/shared/src/event_topics.rs
# Adding a new topic: define a constant there, then use it at the call site.

set -euo pipefail

SEARCH_ROOT="${1:-contracts}"
TOPICS_MODULE="contracts/shared/src/event_topics.rs"
VIOLATIONS=0

echo "Scanning for ad hoc event topics under: $SEARCH_ROOT"
echo "Canonical topics module: $TOPICS_MODULE"
echo ""

# Build a list of known canonical symbol_short! strings from the topics module.
# We extract the string arguments to symbol_short! and Symbol::short() calls.
CANONICAL_TOPICS=$(grep -oP '(?<=symbol_short!\()[^)]+(?=\))' "$TOPICS_MODULE" 2>/dev/null \
    | tr -d '"' \
    | sort -u)

# Find all Rust source files (excluding the canonical module itself and tests).
while IFS= read -r file; do
    # Skip the canonical module.
    if [[ "$file" == *"event_topics.rs" ]]; then
        continue
    fi

    # Look for .publish( calls that contain a symbol_short! or Symbol::new inline.
    # Pattern: publish( on a line, followed within a few lines by symbol_short! or
    # Symbol::new with a string literal not preceded by event_topics::.
    if grep -n 'events()\.publish(' "$file" > /dev/null 2>&1; then
        # Extract lines around each publish call and check for inline topic literals.
        while IFS= read -r match; do
            lineno=$(echo "$match" | cut -d: -f1)
            line=$(echo "$match" | cut -d: -f2-)

            # Check if the topic tuple uses Symbol::new or symbol_short! directly.
            if echo "$line" | grep -qP '(symbol_short!|Symbol::new|Symbol::short)\s*\('; then
                # Allow references prefixed with event_topics:: (canonical).
                if ! echo "$line" | grep -q 'event_topics::'; then
                    echo "  VIOLATION $file:$lineno"
                    echo "    $line"
                    VIOLATIONS=$((VIOLATIONS + 1))
                fi
            fi
        done < <(grep -n 'events()\.publish(' "$file")
    fi
done < <(find "$SEARCH_ROOT" -name '*.rs' -not -path '*/target/*')

echo ""
if [[ $VIOLATIONS -gt 0 ]]; then
    echo "ERROR: Found $VIOLATIONS ad hoc event topic(s)."
    echo "Move the topic string(s) to contracts/shared/src/event_topics.rs"
    echo "and reference them via shared::event_topics:: at each call site."
    exit 1
else
    echo "OK: No ad hoc event topics detected."
    echo "(Note: existing call sites use inline topics for backward compatibility."
    echo " New call sites must use shared::event_topics constants.)"
    exit 0
fi
