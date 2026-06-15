#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
RUST="mise exec rust@1.96.0 -- cargo"

echo "building obol-ffi + cli…"
$RUST build -p obol-ffi -p obol-cli

LIBDIR="$ROOT/target/debug"
LIBNAME="libobol_ffi.$([ "$(uname)" = Darwin ] && echo dylib || echo so)"
export OBOL_LIB="$LIBDIR/$LIBNAME"

SEED="$(mktemp -d)"; trap 'rm -rf "$SEED"' EXIT
cp bindings/testdata/prices.json "$SEED/current.json"
export OBOL_PRICING_DIR="$SEED"

# Normalize any numeric string to Python's shortest round-trip repr of its f64 value.
norm() { python3 -c 'import sys; print(repr(float(sys.stdin.read().strip())))'; }

# Ensure the TS binding's deps (koffi, for the Node consumer) are present.
( cd bindings/typescript && bun install >/dev/null 2>&1 || npm install >/dev/null 2>&1 )

# check <transcript> <dialect>
# Runs all five consumers, normalizes total_usd, asserts equality.
check() {
  local transcript="$1"
  local dialect="$2"

  local rust_total py_total go_total ts_bun ts_node

  rust_total=$($RUST run -q -p obol-cli -- estimate "$transcript" --dialect "$dialect" --json \
    | python3 -c 'import sys,json; print(json.load(sys.stdin)["total_usd"])' | norm)
  py_total=$( (cd bindings/python && PYTHONPATH=. python3 -c \
    "import obol; print(obol.estimate_path('$ROOT/$transcript', dialect='$dialect').total_usd)") | norm)
  go_total=$( (cd bindings/go && CGO_ENABLED=0 go run ./cmd/total "$ROOT/$transcript" "$dialect") | norm)
  ts_bun=$(  (cd bindings/typescript && bun  total.ts "$ROOT/$transcript" "$dialect") | norm)
  ts_node=$( (cd bindings/typescript && node total.ts "$ROOT/$transcript" "$dialect") | norm)

  echo "rust    : $rust_total"
  echo "py      : $py_total"
  echo "go      : $go_total"
  echo "ts(bun) : $ts_bun"
  echo "ts(node): $ts_node"

  if [ "$rust_total" = "$py_total" ] && [ "$py_total" = "$go_total" ] \
     && [ "$go_total" = "$ts_bun" ] && [ "$ts_bun" = "$ts_node" ]; then
    echo "OK: $dialect rust==python==go==ts(bun)==ts(node) ($rust_total)"
  else
    echo "MISMATCH: dialect=$dialect rust=$rust_total py=$py_total go=$go_total ts_bun=$ts_bun ts_node=$ts_node"; exit 1
  fi
}

check bindings/testdata/claude-mini.jsonl   claude
check bindings/testdata/gemini-mini.jsonl   gemini
check bindings/testdata/opencode-mini.json  opencode
check bindings/testdata/copilot-mini.jsonl  copilot
check bindings/testdata/kimi-mini.jsonl     kimi
check bindings/testdata/atif-mini.json      atif
