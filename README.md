# obol

[![CI](https://github.com/prime-radiant-inc/obol/actions/workflows/ci.yml/badge.svg)](https://github.com/prime-radiant-inc/obol/actions/workflows/ci.yml)

Read an AI-agent transcript and estimate what it cost. `obol` parses agent session files,
extracts per-message token usage (with the dedup, cache-bucket, and price-tier handling
naive summers get wrong), and prices it against LiteLLM and OpenRouter snapshots.

The name is the coin paid as a toll — what the run cost you for passage.

## Status

A Rust library (`obol-core`) + CLI (`obol`), plus language bindings. Nine transcript
dialects: **Claude Code, Codex, Pi, Gemini, OpenCode, Copilot, Kimi**, **obol** — our
own usage-sidecar format, a minimal `{"type":"obol.usage", …}` JSONL that in-house
harnesses can emit to get priced without obol learning their transcript format (spec:
[`docs/specs/2026-06-08-obol-usage-sidecar-design.md`](./docs/specs/2026-06-08-obol-usage-sidecar-design.md)) —
and **atif**, the ATIF (Agent Trajectory Interchange Format) `trajectory.json` that
superpowers-evals normalizes every agent's session log into, so obol prices one stable
canonical input instead of re-parsing each agent's raw log.

Bindings reach the core through a small C ABI (`obol-ffi`, a cdylib) and re-type its JSON;
they never re-implement the accounting:

- **Python** (ctypes), **Go** (purego — no cgo, no C compiler), **TypeScript** under both
  **Bun** (`bun:ffi`) and **Node** (`koffi`).

Per-session totals are validated to match `superpowers-evals` exactly on a real corpus, and
all five consumers (Rust CLI, Python, Go, TS/Bun, TS/Node) produce a byte-identical
`total_usd` for the same transcript, across every dialect — see `docs/validation-*.md`.
Design specs and build plans live under `docs/specs/` and `docs/plans/`.

## Install

One `vX.Y.Z` tag publishes every channel (see [`docs/RELEASING.md`](./docs/RELEASING.md)):

| Channel        | Install                                                                  |
| -------------- | ------------------------------------------------------------------------ |
| CLI            | `cargo install obol-cli` (the binary is `obol`)                          |
| Rust library   | `cargo add obol-core`                                                    |
| TypeScript     | `npm install @primeradianthq/obol` (Bun and Node, native libs included)  |
| Python         | `pip install primeradianthq-obol` (imports as `obol`)                    |
| Go             | `go get github.com/prime-radiant-inc/obol-go` (self-contained, embeds the native library) |

## CLI usage

A pricing snapshot is bundled into the binary, so `estimate` works out of the box:

```bash
# Dialect is auto-detected; --dialect overrides
# (atif | claude | codex | pi | gemini | opencode | copilot | kimi | obol).
obol estimate ~/.claude/projects/<…>/<session>.jsonl
obol estimate rollout-….jsonl --dialect codex --json

# Optionally pull fresher prices (LiteLLM + OpenRouter). --as-of defaults to
# the current UTC datetime; pass it explicitly to pin a stamp.
obol refresh
obol refresh --as-of 2026-06-09                  # or 2026-06-09T18:30:00Z
```

Default output is a human total + per-model breakdown. `--json` emits the full
`CostEstimate`, including `unpriced_models` (models with no price entry — surfaced, never a
silent $0), `approximations` (e.g. an assumed standard service tier for Codex), and
`pricing_source` (`bundled` or `local`).

Price-sheet resolution: `$OBOL_PRICING_DIR`, if set, wins absolutely. Otherwise obol uses
the newer (by `as_of`) of the refreshed on-disk snapshot (`$XDG_DATA_HOME/obol`, default
`~/.local/share/obol`) and the embedded one — the bundled sheet is a floor, never a trap.

## Library usage

Every binding exposes the same surface: `estimate_path(path, dialect)` — dialect is
**required** at the API level (auto-detection is a CLI convenience) — plus
`refresh(as_of)` and `version()`. `as_of` must be `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SSZ`
(UTC); the time form lets you stamp multiple refreshes per day. The library has no clock —
the caller supplies the stamp, and anything malformed is rejected before any network or
disk I/O. TypeScript additionally exports
`setPricingDir`/`clearPricingDir` for hermetic pricing in tests.

**If you embed obol, own the refresh story.** The bundled snapshot is frozen at package
time, and prices move faster than code releases. Decide how often your tool refreshes and
where that lives in *your* UX — a `toolx prices --update` subcommand, a staleness warning, a
background job calling `refresh()`. Your users should never have to install `obol-cli` just
to keep your tool's price sheet current.

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

## Acknowledgements

obol's dialect parsers stand on the shoulders of
[AgentsView](https://github.com/kenn-io/agentsview) (MIT, © 2026 Kenn Software LLC). Rather
than guess at the quirks of each agent's transcript format, we reconciled our parsers
against AgentsView's — their careful reverse-engineering of these formats saved us a great
deal of trial and error. Thank you. 🙏

## License

[Apache License 2.0](./LICENSE). See [`NOTICE`](./NOTICE) for attributions.
