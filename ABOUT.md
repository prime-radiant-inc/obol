# obol

> Read an AI-agent transcript and estimate what it cost.

**Family:** obol · **Type:** library · **Lifecycle:** production · **Owner:** mhat

## What it does
A Rust core (`obol-core`) plus CLI that reads agent-run usage in two dialects — **obol**,
a minimal usage-sidecar JSONL that in-house harnesses emit, and **atif**, the ATIF
`trajectory.json` that superpowers-evals normalizes every agent's session log into
(per-agent raw-log parsers were removed in 0.6.0) — extracts per-message token usage
(handling dedup, cache buckets, and price tiers), and prices it against LiteLLM/OpenRouter
snapshots. A small C ABI (`obol-ffi`, a cdylib) lets Python, Go, and TypeScript bindings
re-type the core's JSON without re-implementing the accounting — all consumers produce a
byte-identical `total_usd` for the same transcript.

## How it fits
- Depends on: — (this is the core; no internal dependencies)
- Used by: [obol-go](https://github.com/prime-radiant-inc/obol-go), plus the in-repo
  Python and TypeScript bindings under `bindings/`
- External: LiteLLM and OpenRouter pricing snapshots

## Runtime & data
- Runs: library + CLI (no deployed service)
- Data in: obol usage-sidecar JSONL or ATIF `trajectory.json`
- Data out: per-session USD cost estimate (JSON)

## Links
- Validation: `docs/validation-*.md`
- Specs & plans: `docs/specs/`, `docs/plans/`

<!-- Maintained by the maintaining-project-map skill. Do not hand-edit; regenerated. -->
