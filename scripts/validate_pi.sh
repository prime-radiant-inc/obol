#!/usr/bin/env bash
# Differential validation: obol Pi dialect vs prudence's own per-cell numbers.
#
# For each prudence cell dir (animalia/<contestant>/<seed>/, containing
# result.json + transcript.raw.jsonl.zst), decompress the Pi transcript, run
# obol estimate --dialect pi, and print obol vs prudence (result.json) side by
# side: token buckets + cost. See docs/validation-pi-2026-06-05.md for findings.
#
# Usage:
#   scripts/validate_pi.sh <cell-dir> [<cell-dir> ...]
# Example:
#   scripts/validate_pi.sh \
#     ~/Code/prime/prudence/runs/animalia-codex-vs-pidev/2026-06-03T04-54-43Z/animalia/08b0048a092b/1 \
#     ~/Code/prime/prudence/runs/animalia-openrouter/2026-06-05T05-22-24Z/animalia/68434179a5f9/1
#
# Requires: mise (rust@1.96.0), zstd, python3.
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <cell-dir> [<cell-dir> ...]" >&2
  exit 2
fi

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export OBOL_PRICING_DIR="${OBOL_PRICING_DIR:-$REPO/.pricing}"
AS_OF="${AS_OF:-$(date -u +%Y-%m-%d)}"
CARGO=(mise exec rust@1.96.0 -- cargo)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$OBOL_PRICING_DIR"
echo "== refreshing obol pricing (as-of $AS_OF) =="
( cd "$REPO" && "${CARGO[@]}" run -q -p obol-cli -- refresh --as-of "$AS_OF" )
echo

# Pretty-print prudence's result.json: tokens are nested under "pi" (older
# runner) or "runner" (newer runner); cost is total_cost_usd.
cat > "$TMP/prudence_fmt.py" <<'PY'
import sys, json
d = json.load(open(sys.argv[1]))
spec = d.get("contestant_spec", {})
block = d.get("pi") or d.get("runner") or {}
t = block.get("tokens", {}) or {}
cost = d.get("total_cost_usd")
print("prudence usd=%-12s in=%-8s out=%-8s cr=%-9s cw=%-6s  [%s/%s harness=%s status=%s]" % (
    "%.6f" % cost if cost is not None else "None",
    t.get("input"), t.get("output"), t.get("cache_read"), t.get("cache_write"),
    spec.get("provider"), spec.get("model"), spec.get("harness"), d.get("status")))
PY

cat > "$TMP/obol_fmt.py" <<'PY'
import sys, json
d = json.load(sys.stdin)
t = d["tokens"]
print("obol     usd=%-12s in=%-8s out=%-8s cr=%-9s cw=%-6s  unpriced=%s" % (
    "%.6f" % d["total_usd"], t["input"], t["output"],
    t["cache_read"], t["cache_write"], d["unpriced_models"]))
PY

for cell in "$@"; do
  zst="$cell/transcript.raw.jsonl.zst"
  res="$cell/result.json"
  echo "== $cell =="
  if [ ! -f "$zst" ] || [ ! -f "$res" ]; then
    echo "  SKIP: missing transcript.raw.jsonl.zst or result.json" >&2
    echo
    continue
  fi
  zstd -dcq "$zst" > "$TMP/cell.jsonl"
  ( cd "$REPO" && "${CARGO[@]}" run -q -p obol-cli -- estimate "$TMP/cell.jsonl" --dialect pi --json ) \
    | python3 "$TMP/obol_fmt.py"
  python3 "$TMP/prudence_fmt.py" "$res"
  echo
done
