#!/usr/bin/env bash
# Regenerate the price snapshot bundled into obol-core (crates/obol-core/prices/bundled.json).
#
# Runs `obol refresh` (which fetches BOTH the LiteLLM sheet and the OpenRouter table) into a
# temp dir, then copies the resulting current.json into the crate. The bundled snapshot is a
# point-in-time asset shipped so obol prices out of the box with no `obol refresh`; re-run this
# to refresh it. OpenRouter's third-party rates drift faster than LiteLLM's first-party list
# prices, so treat the bundle as "good enough out of the box" and `refresh` for current numbers.
#
# Usage: scripts/update-bundled-prices.sh [YYYY-MM-DD]   (defaults to today, UTC)
set -euo pipefail
cd "$(dirname "$0")/.."

AS_OF="${1:-$(date -u +%Y-%m-%d)}"
DEST="crates/obol-core/prices/bundled.json"
TMPD="$(mktemp -d)"
trap 'rm -rf "$TMPD"' EXIT

echo "refreshing prices as of $AS_OF (LiteLLM + OpenRouter)…"
OBOL_PRICING_DIR="$TMPD" cargo run -q -p obol-cli --release -- refresh --as-of "$AS_OF"
cp "$TMPD/current.json" "$DEST"

python3 - "$DEST" <<'PY'
import json, sys
path = sys.argv[1]
d = json.load(open(path))
ns = {k: len(v) for k, v in d["namespaces"].items()}
print(f"bundled {path}: namespaces={ns} as_of={d['as_of']}")
if "openrouter" not in d["namespaces"] or not d["namespaces"]["openrouter"]:
    sys.exit("ERROR: openrouter namespace missing or empty — refresh did not include OpenRouter")
PY
