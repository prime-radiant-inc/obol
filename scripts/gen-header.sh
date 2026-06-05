#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mise exec rust@1.96.0 -- cargo run -q -p obol-ffi --example gen_header
