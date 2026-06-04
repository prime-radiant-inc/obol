# obol — design spec (v1)

> 2026-06-04 · Shevek@7998e83e · draft for review
> Lives in `notes/` until the `obol` repo is blessed; then moves to `obol/docs/specs/2026-06-04-obol-design.md`.
> Background: [transcript-cost-tooling-recon.md] (why this exists; what we're NOT building).

## Goal

A Rust library + CLI that reads an agent transcript and estimates what it cost in USD,
correctly handling the per-message token accounting that naive summers get wrong (dedup,
cache buckets, price tiers). v1 supports two dialects — Claude Code and Codex — and is
vetted against the estimates our existing tools already produce on the same files.

## Name

`obol` — the coin paid as a toll/fare. The tool answers "what was the toll for this run."

## Scope

**In v1:**
- Rust core lib (`obol-core`): parse Claude + Codex session files → per-message usage → price → `CostEstimate`.
- `refresh_pricing_tables()` in the library: fetch the LiteLLM price sheet, write to disk.
- CLI (`obol`): `obol estimate <file>` and `obol refresh`.
- Pricing from LiteLLM only (both v1 dialects bill first-party; LiteLLM's first-party prices
  are exact — measured 0% delta vs OpenRouter). Price store is namespace-keyed so OpenRouter
  is an additive change later.

**Out of v1 (recorded so we don't lose it):**
- TS / Go / Python bindings; the C-ABI (`obol-ffi`) layer. Comes after the CLI is vetted.
- OpenRouter price source + Pi/Gemini dialects (Pi bills through OpenRouter; route by
  provenance when added).
- Semantic tool-category mapping (deliberately killed — see recon).
- Structural turn/tool-call extraction beyond what cost needs.
- Any DB / file-watcher / daemon / browser-WASM. obol is a library you call, not a service.

## Architecture

```
obol/                       # its own git repo
  mise.toml                 # pins the rust toolchain (rust is available via mise)
  Cargo.toml                # workspace
  crates/
    obol-core/              # the library
      src/
        lib.rs              # public API: estimate_cost(), refresh_pricing_tables()
        model.rs            # CostEstimate, ModelCost, TokenBuckets, Approximation, Provider
        error.rs            # ObolError
        pricing/
          mod.rs            # PriceStore (namespace -> model -> ModelPrice), lookup + tier
          refresh.rs        # fetch LiteLLM (ureq), normalize to per-million, write to disk
          store.rs          # XDG path resolution, load/save dated snapshots
        transcript/
          mod.rs            # Dialect trait + registry + detect(); MessageUsage type
          claude.rs         # Claude session-file usage extractor
          codex.rs          # Codex rollout usage extractor
        cost.rs             # engine: Vec<MessageUsage> + PriceStore -> CostEstimate
    obol-cli/               # thin CLI over obol-core
  tests/
    fixtures/               # real Claude + Codex sessions copied from coding-agent-files
```

The `Dialect` trait is the extensibility seam for "mirror all of them later." Each agent is
one file: `fn parse(&self, bytes: &[u8]) -> Result<Vec<MessageUsage>>` plus the pricing
namespace it bills through. v1 ships two; the third is a new file, not a refactor.

## Data model

```rust
enum Provider { Anthropic, OpenAI }          // grows with dialects

struct MessageUsage {
    model: String,            // verbatim from the transcript — the price key, not normalized
    provider: Provider,
    namespace: &'static str,  // "litellm" for v1
    input_uncached: u64,
    cache_read: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    output: u64,
    request_input_tokens: u64,// full billed input for this request (for tier selection)
    service_tier: Option<String>,
}

struct CostEstimate {
    total_usd: f64,
    per_model: Vec<ModelCost>,        // { model, provider, tokens, subtotal_usd }
    tokens: TokenBuckets,             // input, output, cache_read, cache_write (summed)
    unpriced_models: Vec<String>,     // no table entry — surfaced, never a silent $0
    approximations: Vec<Approximation>,
    pricing_as_of: String,            // date of the table used
}

enum Approximation {
    UnpricedModel(String),
    AssumedStandardTier,              // service tier not recorded (Codex); assumed standard
    UnknownModelForTurn,              // a Codex turn whose turn_context.model was empty
}
```

The result carries its own caveats because the failure mode here is a confident wrong
number, not a crash.

## Dialect parsing (exact field paths, from real fixtures)

### Claude (`type: assistant` JSONL lines)
*(Reconciled with AgentsView `internal/parser/claude.go` + `claude_test.go`, MIT.)*

- Token extraction only on `type == "assistant"` lines carrying `message.usage`. **Skip**
  (no billing): `user` lines with `isMeta: true`, `isCompactSummary` lines, and
  `progress`/`queue-operation`/`attachment` types. Malformed/truncated lines: count as
  malformed, never bill. `isSidechain` lines **are** billed — subagent tokens are real cost.
- **Two-layer dedup — this is where naive parsers double-count:**
  - *Within a file:* collapse consecutive assistant entries that share the same `message.id`
    into one; the **last** entry in the run owns the usage (streaming snapshots overwrite —
    never sum). A run is closed by `stop_reason: end_turn`; entries after that under the same
    id are a new logical response, not more of the same one.
  - *Across files:* dedup by the ccusage key `message.id:requestId` (note `requestId` is
    **top-level**, not under `message`). The same API call can appear in two session files.
- Per surviving message:
  - `model` ← `message.model` (e.g. `claude-opus-4-7`); provider Anthropic; namespace litellm.
  - `input_uncached` ← `usage.input_tokens`
  - `cache_read` ← `usage.cache_read_input_tokens`
  - cache-write split ← `usage.cache_creation.ephemeral_5m_input_tokens` /
    `ephemeral_1h_input_tokens` (real in fixtures; AgentsView ignores the split and treats
    `cache_creation_input_tokens` flat — obol keeps the split for correct 1h-cache pricing,
    falling back to flat `cache_creation_input_tokens` as 5m when the sub-keys are absent)
  - `output` ← `usage.output_tokens`
  - `request_input_tokens` ← `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`
  - `service_tier` ← `usage.service_tier` (informational for Claude; the table has no Claude
    service-tier variant, so it doesn't move v1 price)
- **Presence, not value:** a usage key present with value `0` means "known zero" and is
  distinct from absent usage. Track coverage by key presence so a genuinely zero-cost turn
  isn't confused with a turn whose data is missing.

### Codex (`rollout-*.jsonl`)
*(Reconciled with AgentsView `internal/parser/codex.go` + `codex_test.go`, MIT. This corrects
my fixture-based first draft, which used one cumulative total + one session model.)*

- Detection: filename `rollout-*.jsonl` (AgentsView is purely filename-based). When obol is
  handed a file directly, treat it as Codex.
- **Per-API-call usage, summed — NOT one cumulative total.** Walk events in order; for each
  `event_msg` with `payload.type == "token_count"`, read `payload.info.last_token_usage`
  (the per-call delta), **skipping** any event whose raw `last_token_usage` JSON is identical
  to the previous one (streaming retransmit). For each:
  - `input_uncached` = `max(input_tokens − cached_input_tokens, 0)`
  - `cache_read` = `cached_input_tokens` (subset of `input_tokens`)
  - `output` = `output_tokens` (already includes `reasoning_output_tokens` — don't add it)
  - cache-write = 0 (OpenAI exposes none)
  - `request_input_tokens` = `input_tokens` for *this call* — so the 272k input tier can be
    applied per call (this is why we sum per-call instead of taking a cumulative total).
  - (The final `info.total_token_usage` is a useful cross-check: it should equal the sum of
    deduped per-call deltas.)
- **Model resolution & mid-session change:** `model` ← the `turn_context.model` in effect at
  the call. `turn_context` events update the running model and it **can change mid-session**;
  an empty `turn_context.model` **clears** it. Stamp each per-call usage with the model in
  effect at that call — do NOT collapse to one session model, or a model switch misprices
  every token. `session_meta.model` is null; never use it.
- **Service tier:** gpt-5.5 has flex/priority/batch tiers at different rates, but the
  transcript doesn't record which was used. v1 assumes the standard tier (codex-tui's default)
  and emits `AssumedStandardTier`.

## Parser provenance & cross-cutting rules

The dialect extractors are reconciled against **AgentsView** (`github.com/kenn-io/agentsview`,
MIT, © 2026 Kenn Software LLC) — we crib their hard-won field paths and edge-case handling
rather than guess from a couple of fixtures. MIT permits this; we keep an attribution note in
the parser source. We take only the cost-relevant slice (usage, model, dedup, line-filtering)
and skip their tool-categorization / relationship / UI layers.

Cross-cutting rules both dialects follow:
- **Malformed/truncated lines** are counted (a `malformed_lines` tally) and never billed.
- **Large integers:** token counts can exceed 2^53; never round-trip usage through `f64`.
  Rust `serde_json` into `u64`/`i64` is safe — just don't deserialize counts as floats.
- **Fork/DAG sessions:** a single Claude file can branch (a re-rolled turn), so the same
  logical turn may appear on multiple branches. v1 sums actual billed assistant turns and
  notes if a file contains forks, rather than silently de-duping branches — abandoned
  branches were still paid for. (Revisit if this proves wrong against the validation corpus.)

## Pricing model

`PriceStore`: `namespace -> model_key -> ModelPrice`. `ModelPrice` holds per-million USD
rates for input / output / cache_read / cache_write, plus optional tier overrides
(`*_above_200k` / `*_above_272k`) and optional `cache_write_above_1hr`, populated faithfully
from the LiteLLM sheet.

- **Routing:** each `MessageUsage` carries its namespace + verbatim model string. Look up
  `store[namespace][model]`. No normalization — the model string is the billing key.
- **Missing model:** push to `unpriced_models`, contribute $0 to the total, never silently
  absorb it.
- **Per-message cost:** select tier by `request_input_tokens` vs the model's boundary; apply
  provider cache semantics (Anthropic: four disjoint buckets incl. 5m/1h cache-write;
  OpenAI: input_uncached + cache_read, no write). Sum into `per_model` and the total.

`refresh_pricing_tables()` (in the library, uses a small blocking HTTP client e.g. `ureq`):
fetch LiteLLM `model_prices_and_context_window.json`, keep chat models with input+output
token costs, convert per-token → per-million, write `litellm-YYYY-MM-DD.json` and update a
`current` pointer under the storage dir. Failure leaves the previous snapshot intact.

## Storage

Price snapshots live under `$XDG_DATA_HOME/obol/` (default `~/.local/share/obol/`),
overridable with `OBOL_PRICING_DIR`. Nothing fancier — dated JSON files + a `current` pointer.

## CLI

- `obol refresh` — pull the LiteLLM sheet to the storage dir; print what changed.
- `obol estimate <path> [--dialect claude|codex] [--json]` — when `--dialect` is absent,
  detect from the first lines' structure (Codex lines carry a top-level `payload` with
  `session_meta`/`response_item`/`event_msg` types; Claude lines carry `message` with
  `type` user/assistant); print a human table by default, or the
  `CostEstimate` as JSON with `--json`. Exit non-zero only on real errors (missing tables,
  malformed file), not on unpriced-model/approximation (those ride in the output).

## Error handling

`ObolError`: `PricingTablesMissing` (tell the user to run `obol refresh`), `UnknownDialect`,
`MalformedTranscript { line }`, `Network` (refresh only). Unpriced models and tier/cache
assumptions are results, not errors.

## Testing & validation

- **Exact-value tests** against real fixtures (copied from `coding-agent-files`): hand-compute
  the dollar figure for one Claude session and one Codex session from the price table, assert
  obol matches. Pricing math is pure functions over fixed inputs, so assertions are exact.
- **Unit tests** for the traps: dedup (repeated message counts once), 5m-vs-1h cache split,
  the 200k/272k tier boundary, unpriced-model-doesn't-silently-zero.
- **Differential validation (acceptance criterion):** run obol over the `coding-agent-files`
  corpus and compare per-session totals against `superpowers-evals/quorum/token_usage.py`
  (which reads the same Claude/Codex session files). For first-party models the two should
  agree within rounding; where they differ, the cause should be obol being *more* correct
  (dedup, per-message tiering, exact cache-TTL) — investigate and document any gap, don't
  paper over it.

## Future (out of scope now, recorded)

- OpenRouter namespace + Pi/Gemini dialects, routed by provenance.
- C-ABI core (`obol-ffi`, cdylib + cbindgen) as the multi-language spine; thin per-language
  wrappers (Python `ctypes`, Go cgo/purego) that re-type the result. JSON at the FFI seam,
  re-typed into idiomatic structs in each language — or a JSON-Schema source of truth that
  codegens the per-language types (keeps JSON on the wire, generates the contract).
- TS: spawn-the-CLI wrapper (runtime-agnostic across Node + Bun) or napi-rs if in-process is
  needed.
- Repo topology when bindings land: monorepo with `bindings/<lang>` subdirs is the default;
  Go may want its own repo (modules + cgo prefer a repo root).
