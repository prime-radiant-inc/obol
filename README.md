# obol

[![CI](https://github.com/prime-radiant-inc/obol/actions/workflows/ci.yml/badge.svg)](https://github.com/prime-radiant-inc/obol/actions/workflows/ci.yml)

Read an AI-agent transcript and estimate what it cost. `obol` parses Claude Code and
Codex session files, extracts per-message token usage (with the dedup, cache-bucket, and
price-tier handling naive summers get wrong), and prices it against a LiteLLM snapshot.

The name is the coin paid as a toll — what the run cost you for passage.

## Status

A Rust library (`obol-core`) + CLI (`obol`), plus language bindings. Three transcript
dialects — **Claude Code, Codex, and Pi** — priced against LiteLLM and OpenRouter snapshots.

Bindings reach the core through a small C ABI (`obol-ffi`, a cdylib) and re-type its JSON;
they never re-implement the accounting:

- **Python** (ctypes), **Go** (cgo), **TypeScript** under both **Bun** (`bun:ffi`) and
  **Node** (`koffi`).

Per-session totals are validated to match `superpowers-evals` exactly on a real corpus, and
all five consumers (Rust CLI, Python, Go, TS/Bun, TS/Node) produce a byte-identical
`total_usd` for the same transcript — see `docs/validation-*.md`. Design specs and build
plans live under `docs/specs/` and `docs/plans/`.

## Build & test

Rust is pinned via [mise](https://mise.jdx.dev) (`mise.toml`). If `cargo` isn't on your
PATH, prefix commands with `mise exec rust@1.96.0 --`.

```bash
cargo build --workspace
cargo test  --workspace -- --test-threads=1   # see note below
```

> **Note:** a few tests set the process-global `OBOL_PRICING_DIR` env var, so the suite
> must run single-threaded (`--test-threads=1`). Running the default parallel harness will
> intermittently fail on the env-var tests. (Threading the dir through as a parameter to
> remove this coupling is tracked as v1.1 polish.)

## Usage

```bash
# 1. Pull the latest price sheet (writes to $XDG_DATA_HOME/obol, or $OBOL_PRICING_DIR).
#    You supply the date stamp — the library has no clock.
obol refresh --as-of 2026-06-04

# 2. Estimate a transcript. Dialect is auto-detected; --dialect overrides.
obol estimate ~/.claude/projects/<…>/<session>.jsonl
obol estimate rollout-….jsonl --dialect codex --json
```

Default output is a human total + per-model breakdown. `--json` emits the full
`CostEstimate`, including `unpriced_models` (models with no price entry — surfaced, never a
silent $0) and `approximations` (e.g. an assumed standard service tier for Codex).

Pricing is stored under `$OBOL_PRICING_DIR` if set, else `$XDG_DATA_HOME/obol`, else
`~/.local/share/obol`. If you run `estimate` before `refresh`, you'll get a clear
"pricing tables not found — run `obol refresh`" error.

## Acknowledgements

obol's dialect parsers stand on the shoulders of
[AgentsView](https://github.com/kenn-io/agentsview) (MIT, © 2026 Kenn Software LLC). Rather
than guess at the quirks of each agent's transcript format, we reconciled our Claude, Codex,
and Pi parsers against AgentsView's — their careful reverse-engineering of these formats
saved us a great deal of trial and error. Thank you. 🙏

## License

[Apache License 2.0](./LICENSE). See [`NOTICE`](./NOTICE) for attributions.
