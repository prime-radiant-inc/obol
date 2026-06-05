#!/usr/bin/env bash
# Differential validation: obol vs superpowers-evals on the same session logs.
#
# Runs obol (live LiteLLM pricing) and superpowers-evals' quorum.token_usage
# (hardcoded list pricing) over each fixture in tests/corpus/ and prints a
# side-by-side per-file comparison of token buckets and cost. See
# docs/validation-2026-06-04.md for the recorded findings.
#
# Requires: mise (rust@1.96.0), python3, and a superpowers-evals checkout with uv.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SEVALS="${SEVALS_DIR:-$HOME/Code/prime/superpowers-evals}"
export OBOL_PRICING_DIR="${OBOL_PRICING_DIR:-$REPO/.pricing}"
AS_OF="${AS_OF:-$(date -u +%Y-%m-%d)}"

CARGO=(mise exec rust@1.96.0 -- cargo)

echo "repo:        $REPO"
echo "sevals:      $SEVALS"
echo "pricing_dir: $OBOL_PRICING_DIR"
echo "as_of:       $AS_OF"
echo

# --- python helpers (written to temp files so heredocs don't tangle with
#     subshell `cd`) -----------------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/obol_fmt.py" <<'PY'
import sys, json
d = json.load(sys.stdin)
t = d["tokens"]
print("obol    usd=%-8s in=%-8s out=%-8s cr=%-8s cw=%-8s unpriced=%s" % (
    round(d["total_usd"], 4), t["input"], t["output"],
    t["cache_read"], t["cache_write"], d["unpriced_models"]))
PY

cat > "$TMP/sevals_fmt.py" <<'PY'
import os
from pathlib import Path
from quorum.token_usage import capture_tokens
u = capture_tokens(os.environ["SEVALS_FAMILY"], [Path(os.environ["SEVALS_FILE"])])
print("sevals  usd=%-8s in=%-8s out=%-8s cr=%-8s cw=%-8s unpriced=%s" % (
    round(u["est_cost_usd"], 4), u["total_input"], u["total_output"],
    u["total_cache_read"], u["total_cache_create"], u.get("has_unpriced_model", False)))
PY

mkdir -p "$OBOL_PRICING_DIR"
echo "== refreshing obol pricing =="
( cd "$REPO" && "${CARGO[@]}" run -q -p obol-cli -- refresh --as-of "$AS_OF" )
echo

obol_line() {  # <file> <dialect>
  ( cd "$REPO" && "${CARGO[@]}" run -q -p obol-cli -- estimate "$1" --dialect "$2" --json ) \
    | python3 "$TMP/obol_fmt.py"
}

sevals_line() {  # <file> <family>
  ( cd "$SEVALS" && SEVALS_FILE="$1" SEVALS_FAMILY="$2" uv run python "$TMP/sevals_fmt.py" )
}

# The corpus of real session logs is NOT shipped in this repo (it contained real
# transcripts). Point CORPUS_DIR at a directory holding `claude/` and `codex/`
# subdirs of `.jsonl` session files to re-run this differential check.
CORPUS_DIR="${CORPUS_DIR:-$REPO/tests/corpus}"
if [ ! -d "$CORPUS_DIR/claude" ] && [ ! -d "$CORPUS_DIR/codex" ]; then
  echo "no corpus found under $CORPUS_DIR (set CORPUS_DIR=/path/to/sessions)" >&2
  exit 1
fi

for pair in "claude:claude" "codex:codex"; do
  dir="${pair%%:*}"; dialect="${pair##*:}"
  for f in "$CORPUS_DIR/$dir"/*.jsonl; do
    [ -e "$f" ] || continue
    echo "== $dir $(basename "$f") =="
    obol_line   "$f" "$dialect"
    sevals_line "$f" "$dir"
    echo
  done
done
