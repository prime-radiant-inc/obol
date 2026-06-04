# obol

Read an AI-agent transcript and estimate what it cost. `obol` parses Claude Code and
Codex session files, extracts per-message token usage (with the dedup, cache-bucket, and
price-tier handling naive summers get wrong), and prices it against a LiteLLM snapshot.

The name is the coin paid as a toll — what the run cost you for passage.

## Status

v1: a Rust library (`obol-core`) + CLI (`obol`). Two dialects (Claude Code, Codex),
LiteLLM pricing. Language bindings (TS/Python/Go) are deliberately deferred until the CLI
is proven. See `docs/specs/2026-06-04-obol-design.md` for scope and `docs/plans/` for the
build plan.

Per-session totals are validated to match `superpowers-evals` exactly on a real corpus —
see `docs/validation-2026-06-04.md`.

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

## License

MIT. The dialect parsers are reconciled against
[AgentsView](https://github.com/kenn-io/agentsview) (MIT, © 2026 Kenn Software LLC).
