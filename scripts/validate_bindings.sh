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
T="bindings/testdata/claude-mini.jsonl"

# Normalize any numeric string to Python's shortest round-trip repr of its f64 value.
norm() { python3 -c 'import sys; print(repr(float(sys.stdin.read().strip())))'; }

rust_total=$($RUST run -q -p obol-cli -- estimate "$T" --dialect claude --json \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["total_usd"])' | norm)
py_total=$( (cd bindings/python && PYTHONPATH=. python3 -c \
  "import obol; print(obol.estimate_path('$ROOT/$T', dialect='claude').total_usd)") | norm)
go_total=$( (cd bindings/go && go run ./cmd/total "$ROOT/$T" claude) | norm)

echo "rust : $rust_total"
echo "py   : $py_total"
echo "go   : $go_total"

if [ "$rust_total" = "$py_total" ] && [ "$py_total" = "$go_total" ]; then
  echo "OK: rust == python == go total_usd ($rust_total)"
else
  echo "MISMATCH: rust=$rust_total python=$py_total go=$go_total"; exit 1
fi
